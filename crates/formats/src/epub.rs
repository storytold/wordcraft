//! Reflowable EPUB 3 export.
//!
//! An EPUB is an OCF ZIP package: an uncompressed `mimetype` entry first, a container that points
//! at the package document, XHTML content documents, an OPF manifest and spine, and a navigation
//! document. The flow model gives paragraphs, headings, lists, tables, links, character formatting
//! and pictures. Page layout (sections, headers and footers, floating shapes) has no meaning in a
//! reflowable book and is left out. Content the flow model cannot carry — footnotes, endnotes,
//! comments and text boxes — is named on a final "Conversion notes" page instead of vanishing.
//!
//! Written from the public EPUB 3.3, OCF and XHTML specifications; no code or data from another
//! word processor is used. Pictures that EPUB core media types do not take (BMP, WebP) are
//! re-encoded as PNG with a bounded decoder.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write as _};
use std::sync::Arc;

use wordcraft_doc::{Document, PartKind};

use crate::model::{self, Cell, FBlock, FTable, Fmt, Img, Inline, Kind, MAX_DEPTH, Para};

/// Most pictures one export packages; further pictures are left out and named in the conversion
/// notes (a hostile document cannot make the writer grow without bound).
const MAX_IMAGES: usize = 10_000;

/// EPUB export options. The defaults take the document's own properties; a caller can override
/// them (the tests use this to pin dates and identifiers).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpubOptions {
    pub title: Option<String>,
    pub author: Option<String>,
    /// BCP 47 language tag; the document's default run language when absent.
    pub language: Option<String>,
    /// Package identifier, usually a `urn:uuid:`; a content-derived `urn:wordcraft:` otherwise.
    pub identifier: Option<String>,
    /// `dcterms:modified` as `CCYY-MM-DDThh:mm:ssZ`; the document's last save when absent.
    pub modified: Option<String>,
}

/// Export `doc` as a reflowable EPUB 3 package.
pub fn export(doc: &Document) -> Result<Vec<u8>, String> {
    export_with(doc, &EpubOptions::default())
}

/// [`export`] with metadata overrides.
pub fn export_with(doc: &Document, opts: &EpubOptions) -> Result<Vec<u8>, String> {
    let flow = model::from_doc(doc);
    let language = language(doc, opts);
    let title = book_title(doc, opts, &flow.blocks);
    let mut ctx = Ctx::default();
    let ranges = chapter_ranges(&flow.blocks);
    // Internal links name a bookmark, not the file that will hold it; EPUB splits the book into
    // one document per Heading 1, so record where each bookmark lands before rendering any body.
    for (index, (start, end)) in ranges.iter().copied().enumerate() {
        let file = format!("chapter{}.xhtml", index + 1);
        collect_anchors(flow.blocks.get(start..end).unwrap_or_default(), &file, &mut ctx.anchors, 0);
    }
    let mut chapters = Vec::new();
    for (index, (start, end)) in ranges.iter().copied().enumerate() {
        let file = format!("chapter{}.xhtml", index + 1);
        let chapter_title = chapter_title(&flow.blocks[start..end]).unwrap_or_else(|| title.clone());
        let mut body = String::new();
        render_blocks(&flow.blocks[start..end], &mut ctx, &file, &mut body, 0);
        if body.trim().is_empty() {
            body.push_str("<p>&#160;</p>\n");
        }
        let mathml = body.contains("<math");
        chapters.push(Chapter { file, title: chapter_title, body, mathml });
    }

    ctx.omitted.footnotes = doc.parts.values().filter(|p| p.kind == PartKind::Footnote).count();
    ctx.omitted.endnotes = doc.parts.values().filter(|p| p.kind == PartKind::Endnote).count();
    ctx.omitted.text_boxes = doc.parts.values().filter(|p| p.kind == PartKind::TextBox).count();
    ctx.omitted.comments = doc.comments.len();

    if ctx.omitted.any() {
        let file = "notes.xhtml".to_string();
        chapters.push(Chapter { file: file.clone(), title: "Conversion notes".to_string(), body: conversion_notes(&ctx.omitted), mathml: false });
        ctx.nav.push(NavEntry { file, anchor: String::new(), title: "Conversion notes".to_string(), level: 1 });
    }
    if ctx.nav.is_empty()
        && let Some(first) = chapters.first()
    {
        ctx.nav.push(NavEntry { file: first.file.clone(), anchor: String::new(), title: title.clone(), level: 1 });
    }

    let identifier = opts
        .identifier
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| content_identifier(&title, &language, &chapters, &ctx.images));
    let modified = modified(doc, opts);
    let author =
        opts.author.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| doc.core.creator.trim().to_string());
    let opf = package_document(&PackageParts {
        title: &title,
        author: &author,
        language: &language,
        identifier: &identifier,
        modified: &modified,
        chapters: &chapters,
        images: &ctx.images,
        meta: &flow.meta,
    });
    let nav = nav_document(&language, &contents_title(&language), &ctx.nav);
    let container = CONTAINER;

    let zerr = |e: zip::result::ZipError| format!("EPUB: {e}");
    let ioerr = |e: std::io::Error| format!("EPUB: {e}");
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // OCF requires `mimetype` first, uncompressed, with no extra field.
    z.start_file("mimetype", stored).map_err(zerr)?;
    z.write_all(b"application/epub+zip").map_err(ioerr)?;
    for (name, data) in [
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/style.css", STYLE.as_bytes()),
    ] {
        z.start_file(name, deflated).map_err(zerr)?;
        z.write_all(data).map_err(ioerr)?;
    }
    for chapter in &chapters {
        let name = format!("OEBPS/{}", chapter.file);
        z.start_file(name, deflated).map_err(zerr)?;
        z.write_all(chapter_xhtml(&language, &chapter.title, &chapter.body).as_bytes()).map_err(ioerr)?;
    }
    for image in &ctx.images.items {
        let name = format!("OEBPS/{}", image.name);
        z.start_file(name, stored).map_err(zerr)?;
        z.write_all(&image.data).map_err(ioerr)?;
    }
    let cursor = z.finish().map_err(zerr)?;
    Ok(cursor.into_inner())
}

/// The OCF container: one package document at `OEBPS/content.opf`.
const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\
<rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles>\
</container>\n";

/// The book's stylesheet. Reflowable books keep the reader's page and typography choices; this
/// only sets the structure EPUB needs to render well.
const STYLE: &str = "\
body { font-family: serif; line-height: 1.5; margin: 5%; }\n\
h1, h2, h3, h4, h5, h6 { line-height: 1.2; margin: 1.2em 0 0.5em; }\n\
p { margin: 0 0 0.6em; }\n\
blockquote { margin: 0.5em 2em; font-style: italic; color: #404040; }\n\
pre { background: #f5f5f5; padding: 0.5em; white-space: pre-wrap; font-family: monospace; }\n\
code { font-family: monospace; }\n\
table { border-collapse: collapse; margin: 0.75em 0; }\n\
td, th { border: 1px solid #999; padding: 4px 6px; vertical-align: top; }\n\
table.borderless td, table.borderless th { border: 0; }\n\
img { max-width: 100%; height: auto; }\n\
hr { border: 0; border-top: 1px solid #999; margin: 1em 0; }\n\
.toc { list-style: none; padding-left: 0; }\n\
.toc-level-2 { margin-left: 1.5em; }\n\
.toc-level-3 { margin-left: 3em; }\n\
.missing-image, .figure { font-style: italic; color: #666; }\n\
.conversion-notes { border-top: 1px solid #999; margin-top: 2em; padding-top: 1em; }\n";

#[derive(Default)]
struct Ctx {
    images: Images,
    nav: Vec<NavEntry>,
    omitted: Omitted,
    next_heading: usize,
    /// Every bookmark target in the book, mapped to the content document that carries it.
    anchors: BTreeMap<String, String>,
}

struct Chapter {
    file: String,
    title: String,
    body: String,
    /// The content document holds MathML, which its manifest item must declare.
    mathml: bool,
}

struct NavEntry {
    file: String,
    /// Empty for a whole-file target.
    anchor: String,
    title: String,
    level: u8,
}

#[derive(Default)]
struct Omitted {
    footnotes: usize,
    endnotes: usize,
    comments: usize,
    text_boxes: usize,
    pictures: usize,
    figures: usize,
}

impl Omitted {
    fn any(&self) -> bool {
        self.footnotes + self.endnotes + self.comments + self.text_boxes + self.pictures + self.figures > 0
    }
}

struct ImageItem {
    name: String,
    media_type: &'static str,
    data: Arc<Vec<u8>>,
}

#[derive(Default)]
struct Images {
    items: Vec<ImageItem>,
    /// FNV-1a of the bytes → item indices, so the same picture is packaged once.
    by_hash: BTreeMap<u64, Vec<usize>>,
}

impl Images {
    /// Package `img` and return the file name to reference, or `None` when it can't be included.
    fn add(&mut self, img: &Img) -> Option<String> {
        let (data, ext, media_type) = normalized_image(img)?;
        let hash = fnv1a(&data);
        if let Some(index) = self
            .by_hash
            .get(&hash)
            .and_then(|v| v.iter().copied().find(|i| self.items.get(*i).is_some_and(|item| item.data.as_slice() == data.as_slice())))
        {
            return self.items.get(index).map(|item| item.name.clone());
        }
        if self.items.len() >= MAX_IMAGES {
            return None;
        }
        let name = format!("images/img{}.{}", self.items.len() + 1, ext);
        self.items.push(ImageItem { name: name.clone(), media_type, data });
        self.by_hash.entry(hash).or_default().push(self.items.len() - 1);
        Some(name)
    }
}

/// The bytes, extension and media type to package for `img`. Formats outside EPUB's core media
/// types are decoded with the bounded decoder and re-encoded as PNG; `None` when they can't be.
fn normalized_image(img: &Img) -> Option<(Arc<Vec<u8>>, &'static str, &'static str)> {
    match img.ext.to_ascii_lowercase().as_str() {
        "png" => Some((img.data.clone(), "png", "image/png")),
        "jpg" | "jpeg" => Some((img.data.clone(), "jpg", "image/jpeg")),
        "gif" => Some((img.data.clone(), "gif", "image/gif")),
        "svg" => Some((img.data.clone(), "svg", "image/svg+xml")),
        _ => {
            let decoded = model::decode_bounded(&img.data)?;
            let mut png = Vec::new();
            decoded.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
            Some((Arc::new(png), "png", "image/png"))
        }
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A content-derived identifier, stable for the same document: an EPUB identifier only has to be
/// unique, not a real UUID.
fn content_identifier(title: &str, language: &str, chapters: &[Chapter], images: &Images) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    feed(title.as_bytes());
    feed(language.as_bytes());
    for chapter in chapters {
        feed(chapter.file.as_bytes());
        feed(chapter.body.as_bytes());
    }
    for image in &images.items {
        feed(image.name.as_bytes());
        feed(&image.data);
    }
    format!("urn:wordcraft:{h:016x}")
}

fn book_title(doc: &Document, opts: &EpubOptions, blocks: &[FBlock]) -> String {
    let candidate: Option<String> = opts
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| Some(doc.core.title.trim()).filter(|s| !s.is_empty()).map(str::to_string))
        .or_else(|| {
            blocks.iter().find_map(|b| match b {
                FBlock::Para(p) if matches!(p.kind, Kind::Title | Kind::Heading(1)) => {
                    let t = p.text().trim().to_string();
                    (!t.is_empty()).then_some(t)
                }
                _ => None,
            })
        });
    candidate.unwrap_or_else(|| "Untitled".to_string())
}

fn language(doc: &Document, opts: &EpubOptions) -> String {
    let raw = opts.language.as_deref().or(doc.styles.default_chr.lang.as_deref()).unwrap_or("en");
    let cleaned: String = raw.trim().replace('_', "-").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    if valid_language_tag(&cleaned) { cleaned } else { "en".to_string() }
}

/// Whether `tag` is a well-formed BCP 47 tag: a 2–8-letter primary subtag, then 1–8-character
/// alphanumeric subtags, with at least one subtag after every singleton.
fn valid_language_tag(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let Some(primary) = parts.next() else { return false };
    if !(2..=8).contains(&primary.len()) || !primary.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        if part.is_empty() || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return false;
        }
        if part.len() == 1 && parts.peek().is_none() {
            return false;
        }
    }
    true
}

fn modified(doc: &Document, opts: &EpubOptions) -> String {
    opts.modified.as_deref().and_then(w3cdtf).or_else(|| w3cdtf(doc.core.modified.trim())).unwrap_or_else(crate::odt::now_iso)
}

/// Normalise `s` to EPUB's `CCYY-MM-DDThh:mm:ssZ` when it starts with a valid W3C date-time.
fn w3cdtf(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.len() < 19
        || !b[0..4].iter().all(u8::is_ascii_digit)
        || b[4] != b'-'
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || b[7] != b'-'
        || !b[8..10].iter().all(u8::is_ascii_digit)
        || b[10] != b'T'
        || !b[11..13].iter().all(u8::is_ascii_digit)
        || b[13] != b':'
        || !b[14..16].iter().all(u8::is_ascii_digit)
        || b[16] != b':'
        || !b[17..19].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| s.get(range).and_then(|v| v.parse::<u32>().ok());
    let (month, day) = (number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(format!("{}Z", s.get(..19)?))
}

/// The table of contents' title, in the book's language where that word is known here.
fn contents_title(language: &str) -> String {
    let base = language.split(['-', '_']).next().unwrap_or("en").to_ascii_lowercase();
    let title = match base.as_str() {
        "ar" => "المحتويات",
        "bg" => "Съдържание",
        "cs" => "Obsah",
        "da" => "Indhold",
        "de" => "Inhalt",
        "es" => "Índice",
        "fi" => "Sisällys",
        "fr" => "Table des matières",
        "he" => "תוכן העניינים",
        "hr" => "Sadržaj",
        "hu" => "Tartalomjegyzék",
        "it" => "Indice",
        "ja" => "目次",
        "ko" => "목차",
        "nb" | "no" => "Innhold",
        "nl" => "Inhoud",
        "pl" => "Spis treści",
        "pt" => "Sumário",
        "ro" => "Cuprins",
        "ru" => "Содержание",
        "sr" => {
            if language.to_ascii_lowercase().contains("latn") {
                "Sadržaj"
            } else {
                "Садржај"
            }
        }
        "sv" => "Innehåll",
        "tr" => "İçindekiler",
        "uk" => "Зміст",
        "vi" => "Mục lục",
        "zh" => {
            let lower = language.to_ascii_lowercase();
            if lower.contains("hant") || lower.contains("-tw") || lower.contains("-hk") { "目錄" } else { "目录" }
        }
        _ => "Contents",
    };
    title.to_string()
}

/// The `(start, end)` block ranges of the content documents: a new one starts at each top-level
/// Heading 1.
fn chapter_ranges(blocks: &[FBlock]) -> Vec<(usize, usize)> {
    if blocks.is_empty() {
        return vec![(0, 0)];
    }
    let mut starts = vec![0usize];
    for (i, block) in blocks.iter().enumerate().skip(1) {
        if matches!(block, FBlock::Para(p) if matches!(p.kind, Kind::Heading(1))) {
            starts.push(i);
        }
    }
    let mut ranges = Vec::new();
    for (k, start) in starts.iter().copied().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(blocks.len());
        if start < end {
            ranges.push((start, end));
        }
    }
    if ranges.is_empty() {
        ranges.push((0, blocks.len()));
    }
    ranges
}

fn chapter_title(blocks: &[FBlock]) -> Option<String> {
    blocks.iter().find_map(|block| match block {
        FBlock::Para(p) if matches!(p.kind, Kind::Title | Kind::Heading(1)) => {
            let title = p.text().trim().to_string();
            (!title.is_empty()).then_some(title)
        }
        _ => None,
    })
}

fn heading_level(kind: Kind) -> Option<u8> {
    match kind {
        Kind::Title => Some(1),
        Kind::Heading(n) => Some(n.clamp(1, 6)),
        _ => None,
    }
}

/// Record where every bookmark in `blocks` will be written: the first chapter that carries it.
fn collect_anchors(blocks: &[FBlock], chapter: &str, anchors: &mut BTreeMap<String, String>, depth: usize) {
    for block in blocks {
        match block {
            FBlock::Para(p) => {
                for inline in &p.inlines {
                    if let Inline::Anchor(name) = inline {
                        anchors.entry(anchor_id(name)).or_insert_with(|| chapter.to_string());
                    }
                }
            }
            FBlock::Table(table) if depth < MAX_DEPTH => {
                for row in &table.rows {
                    for cell in row {
                        collect_anchors(&cell.blocks, chapter, anchors, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }
}

fn render_blocks(blocks: &[FBlock], ctx: &mut Ctx, chapter: &str, out: &mut String, depth: usize) {
    let mut i = 0;
    while let Some(block) = blocks.get(i) {
        match block {
            FBlock::Table(table) => {
                if depth < MAX_DEPTH {
                    render_table(table, ctx, chapter, out, depth);
                }
                i += 1;
            }
            FBlock::Para(p) if p.list.is_some() && heading_level(p.kind).is_none() => {
                // Nested lists from consecutive list paragraphs, as in the HTML writer. A heading
                // keeps its heading markup even when the document numbers it; the flow model
                // carries only the list membership, not the generated label, so the heading is
                // written without a list marker.
                let mut stack: Vec<(bool, bool)> = Vec::new(); // (ordered, open <li>)
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    if heading_level(q.kind).is_some() {
                        break;
                    }
                    let Some(list) = q.list else { break };
                    let level = list.level.min(8) as usize;
                    while stack.len() > level + 1 {
                        close_list(&mut stack, out);
                    }
                    if stack.len() == level + 1 && stack.last().is_some_and(|s| s.0 != list.ordered) {
                        close_list(&mut stack, out);
                    }
                    while stack.len() < level + 1 {
                        if let Some(top) = stack.last_mut()
                            && !top.1
                        {
                            out.push_str("<li>");
                            top.1 = true;
                        }
                        out.push_str(if list.ordered { "<ol>" } else { "<ul>" });
                        stack.push((list.ordered, false));
                    }
                    if let Some(top) = stack.last_mut() {
                        if top.1 {
                            out.push_str("</li>\n");
                        }
                        top.1 = true;
                    }
                    out.push_str(&format!("<li{}>", para_attrs(q)));
                    render_inlines(&q.inlines, ctx, chapter, out);
                    i += 1;
                }
                while !stack.is_empty() {
                    close_list(&mut stack, out);
                }
                out.push('\n');
            }
            FBlock::Para(p) if p.kind == Kind::Code => {
                out.push_str("<pre><code>");
                let mut first = true;
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    if q.kind != Kind::Code || q.list.is_some() {
                        break;
                    }
                    if !first {
                        out.push('\n');
                    }
                    first = false;
                    out.push_str(&xml_text(&q.text()));
                    i += 1;
                }
                out.push_str("</code></pre>\n");
            }
            FBlock::Para(p) if p.kind == Kind::Quote => {
                out.push_str("<blockquote>\n");
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    if q.kind != Kind::Quote || q.list.is_some() {
                        break;
                    }
                    out.push_str(&format!("<p{}>", para_attrs(q)));
                    render_inlines(&q.inlines, ctx, chapter, out);
                    out.push_str("</p>\n");
                    i += 1;
                }
                out.push_str("</blockquote>\n");
            }
            FBlock::Para(p) if p.kind == Kind::Rule => {
                out.push_str("<hr/>\n");
                i += 1;
            }
            FBlock::Para(p) => {
                if let Some(level) = heading_level(p.kind) {
                    let id = if depth == 0 && level <= 3 {
                        ctx.next_heading += 1;
                        let id = format!("wc-heading-{}", ctx.next_heading);
                        let title = p.text().trim().to_string();
                        ctx.nav.push(NavEntry {
                            file: chapter.to_string(),
                            anchor: id.clone(),
                            title: if title.is_empty() { "Section".to_string() } else { title },
                            level,
                        });
                        Some(id)
                    } else {
                        None
                    };
                    let id_attr = id.map(|id| format!(" id=\"{}\"", xml_attr(&id))).unwrap_or_default();
                    out.push_str(&format!("<h{level}{id_attr}{}>", para_attrs(p)));
                    render_inlines(&p.inlines, ctx, chapter, out);
                    out.push_str(&format!("</h{level}>\n"));
                } else {
                    out.push_str(&format!("<p{}>", para_attrs(p)));
                    if p.inlines.is_empty() {
                        out.push_str("&#160;");
                    } else {
                        render_inlines(&p.inlines, ctx, chapter, out);
                    }
                    out.push_str("</p>\n");
                }
                i += 1;
            }
        }
    }
}

fn close_list(stack: &mut Vec<(bool, bool)>, out: &mut String) {
    if let Some((ordered, li_open)) = stack.pop() {
        if li_open {
            out.push_str("</li>");
        }
        out.push_str(if ordered { "</ol>" } else { "</ul>" });
    }
}

fn render_table(table: &FTable, ctx: &mut Ctx, chapter: &str, out: &mut String, depth: usize) {
    out.push_str(if table.borderless { "<table class=\"borderless\">\n<tbody>\n" } else { "<table>\n<tbody>\n" });
    for row in &table.rows {
        out.push_str("<tr>");
        for cell in row {
            if cell.covered {
                continue;
            }
            let tag = if cell.header { "th" } else { "td" };
            let mut attrs = String::new();
            if cell.colspan > 1 {
                let _ = write!(attrs, " colspan=\"{}\"", cell.colspan.min(1000));
            }
            if cell.rowspan > 1 {
                let _ = write!(attrs, " rowspan=\"{}\"", cell.rowspan.min(1000));
            }
            if let Some(shading) = cell.shading {
                let _ = write!(attrs, " style=\"background-color:#{}\"", shading.hex());
            }
            out.push_str(&format!("<{tag}{attrs}>"));
            render_cell(cell, ctx, chapter, out, depth);
            out.push_str(&format!("</{tag}>"));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n");
}

fn render_cell(cell: &Cell, ctx: &mut Ctx, chapter: &str, out: &mut String, depth: usize) {
    if cell.blocks.is_empty() {
        out.push_str("&#160;");
        return;
    }
    if let [FBlock::Para(p)] = cell.blocks.as_slice()
        && p.kind == Kind::Normal
        && p.list.is_none()
        && !p.page_break
    {
        render_inlines(&p.inlines, ctx, chapter, out);
    } else {
        render_blocks(&cell.blocks, ctx, chapter, out, depth + 1);
    }
}

/// The base text in the run's formatting, then its phonetic guide inside the same tags
/// (`<ruby>base<rt>guide</rt></ruby>`, EPUB 3 is XHTML5 so ruby is real markup).
fn render_ruby(base: &str, ruby: &str, fmt: &Fmt, ctx: &Ctx, chapter: &str, out: &mut String) {
    let mut open = String::new();
    let mut close: Vec<&'static str> = Vec::new();
    if let Some(href) = fmt.link.as_deref().filter(|l| wordcraft_doc::link::is_safe_href(l)).and_then(|link| link_href(link, chapter, &ctx.anchors)) {
        open.push_str(&format!("<a href=\"{}\">", xml_attr(&href)));
        close.push("</a>");
    }
    let mut tag = |o: &str, c: &'static str| {
        open.push_str(o);
        close.push(c);
    };
    if fmt.bold {
        tag("<strong>", "</strong>");
    }
    if fmt.italic {
        tag("<em>", "</em>");
    }
    if fmt.underline {
        tag("<u>", "</u>");
    }
    if fmt.strike {
        tag("<s>", "</s>");
    }
    if fmt.sup {
        tag("<sup>", "</sup>");
    } else if fmt.sub {
        tag("<sub>", "</sub>");
    }
    if fmt.code {
        tag("<code>", "</code>");
    }
    out.push_str(&open);
    out.push_str("<ruby>");
    for (k, line) in base.split('\n').enumerate() {
        if k > 0 {
            out.push_str("<br/>");
        }
        out.push_str(&xml_text(line));
    }
    out.push_str("<rt>");
    for (k, line) in ruby.split('\n').enumerate() {
        if k > 0 {
            out.push_str("<br/>");
        }
        out.push_str(&xml_text(line));
    }
    out.push_str("</rt></ruby>");
    for c in close.iter().rev() {
        out.push_str(c);
    }
}

fn render_inlines(inlines: &[Inline], ctx: &mut Ctx, chapter: &str, out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text, fmt) => render_text(text, fmt, ctx, chapter, out),
            Inline::Image(img) => match ctx.images.add(img) {
                Some(name) => {
                    let alt = xml_attr(&img.alt);
                    out.push_str(&format!("<img src=\"{name}\" alt=\"{alt}\""));
                    if let Some(w) = pixels(img.w) {
                        let _ = write!(out, " width=\"{w}\"");
                    }
                    if let Some(h) = pixels(img.h) {
                        let _ = write!(out, " height=\"{h}\"");
                    }
                    out.push_str("/>");
                }
                None => {
                    ctx.omitted.pictures += 1;
                    out.push_str("<span class=\"missing-image\">");
                    if img.alt.trim().is_empty() {
                        out.push_str("Image not included");
                    } else {
                        out.push_str(&xml_text(&img.alt));
                    }
                    out.push_str("</span>");
                }
            },
            Inline::Anchor(anchor) => out.push_str(&format!("<a id=\"{}\"></a>", xml_attr(&anchor_id(anchor)))),
            Inline::Figure(alt) => {
                ctx.omitted.figures += 1;
                out.push_str("<span class=\"figure\">");
                if alt.trim().is_empty() {
                    out.push_str("Graphic not included");
                } else {
                    out.push_str(&xml_text(alt));
                }
                out.push_str("</span>");
            }
            // MathML is XML, and EPUB 3 readers may render it. A reader without MathML still
            // gets the equation's text in the annotation.
            Inline::Equation { linear, display } => {
                let mathml = crate::mathml::linear_to_mathml(linear, *display);
                out.push_str(&strip_invalid_xml(&mathml));
            }
            // EPUB 3 is XHTML5: ruby is real markup there (base text with its phonetic guide),
            // inside the run's own tags.
            Inline::Ruby { base, ruby, fmt } => render_ruby(base, ruby, fmt, ctx, chapter, out),
        }
    }
}

fn render_text(text: &str, fmt: &Fmt, ctx: &Ctx, chapter: &str, out: &mut String) {
    let mut open = String::new();
    let mut close: Vec<&'static str> = Vec::new();
    if let Some(href) = fmt.link.as_deref().filter(|l| wordcraft_doc::link::is_safe_href(l)).and_then(|link| link_href(link, chapter, &ctx.anchors)) {
        open.push_str(&format!("<a href=\"{}\">", xml_attr(&href)));
        close.push("</a>");
    }
    let mut tag = |o: &str, c: &'static str| {
        open.push_str(o);
        close.push(c);
    };
    if fmt.bold {
        tag("<strong>", "</strong>");
    }
    if fmt.italic {
        tag("<em>", "</em>");
    }
    if fmt.underline {
        tag("<u>", "</u>");
    }
    if fmt.strike {
        tag("<s>", "</s>");
    }
    if fmt.sup {
        tag("<sup>", "</sup>");
    } else if fmt.sub {
        tag("<sub>", "</sub>");
    }
    if fmt.code {
        tag("<code>", "</code>");
    }
    let mut css = Vec::new();
    if let Some(color) = fmt.color {
        css.push(format!("color:#{}", color.hex()));
    }
    if let Some(background) = fmt.background {
        css.push(format!("background-color:#{}", background.hex()));
    }
    if let Some(size) = fmt.size.filter(|v| v.is_finite()) {
        css.push(format!("font-size:{}pt", trim_num(size.clamp(1.0, 1000.0))));
    }
    if let Some(families) = fmt.font.as_deref().and_then(css_families) {
        css.push(format!("font-family:{families}"));
    }
    if !css.is_empty() {
        open.push_str(&format!("<span style=\"{}\">", css.join(";")));
        close.push("</span>");
    }
    out.push_str(&open);
    for (k, line) in text.split('\n').enumerate() {
        if k > 0 {
            out.push_str("<br/>");
        }
        out.push_str(&xml_text(line));
    }
    for c in close.iter().rev() {
        out.push_str(c);
    }
}

fn para_attrs(p: &Para) -> String {
    let mut css: Vec<String> = Vec::new();
    if let Some(align) = p.align {
        match align {
            wordcraft_doc::Align::Center => css.push("text-align:center".into()),
            wordcraft_doc::Align::Right => css.push("text-align:right".into()),
            wordcraft_doc::Align::Justify | wordcraft_doc::Align::Distribute => css.push("text-align:justify".into()),
            wordcraft_doc::Align::Left => {}
        }
    }
    if p.page_break {
        css.push("break-before:page".into());
    }
    if let Some(v) = p.spacing.before.filter(|v| v.is_finite()) {
        css.push(format!("margin-top:{}pt", trim_num(v.clamp(0.0, 2000.0))));
    }
    if let Some(v) = p.spacing.after.filter(|v| v.is_finite()) {
        css.push(format!("margin-bottom:{}pt", trim_num(v.clamp(0.0, 2000.0))));
    }
    if let Some(v) = p.spacing.left.filter(|v| v.is_finite()) {
        css.push(format!("margin-left:{}pt", trim_num(v.clamp(-2000.0, 2000.0))));
    }
    if let Some(v) = p.spacing.right.filter(|v| v.is_finite()) {
        css.push(format!("margin-right:{}pt", trim_num(v.clamp(-2000.0, 2000.0))));
    }
    if let Some(v) = p.spacing.first.filter(|v| v.is_finite()) {
        css.push(format!("text-indent:{}pt", trim_num(v.clamp(-2000.0, 2000.0))));
    }
    if css.is_empty() { String::new() } else { format!(" style=\"{}\"", css.join(";")) }
}

fn css_families(fonts: &str) -> Option<String> {
    let mut out = Vec::new();
    for font in fonts.split(',') {
        let name: String = font.trim().chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.')).collect();
        let name = name.trim();
        if !name.is_empty() {
            out.push(format!("'{name}'"));
        }
    }
    (!out.is_empty()).then(|| out.join(", "))
}

fn trim_num(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn pixels(points: f32) -> Option<u32> {
    let px = (points / 0.75).round();
    if px.is_finite() && px >= 1.0 { Some(px.clamp(1.0, 10_000.0) as u32) } else { None }
}

/// The `href` for a link written in `here`: internal fragments name the content document that
/// carries the bookmark. `None` when a fragment names no bookmark in the book, so the link is
/// written as plain text.
fn link_href(href: &str, here: &str, anchors: &BTreeMap<String, String>) -> Option<String> {
    match href.strip_prefix('#') {
        Some(fragment) => {
            let id = anchor_id(fragment);
            let file = anchors.get(&id)?;
            Some(if file == here { format!("#{id}") } else { format!("{file}#{id}") })
        }
        None => Some(href.to_string()),
    }
}

/// An HTML id that can go into XHTML: whitespace and control characters become underscores.
fn anchor_id(id: &str) -> String {
    id.chars().map(|c| if c.is_whitespace() || c.is_control() { '_' } else { c }).collect()
}

fn conversion_notes(omitted: &Omitted) -> String {
    let mut out = String::from(
        "<section class=\"conversion-notes\">\n<h1>Conversion notes</h1>\n\
<p>This EPUB was exported from WordCraft. Reflowable EPUB has no pages, headers, footers or floating layout.</p>\n<ul>\n",
    );
    for (count, label) in [
        (omitted.footnotes, "Footnotes not included"),
        (omitted.endnotes, "Endnotes not included"),
        (omitted.comments, "Comments not included"),
        (omitted.text_boxes, "Text boxes not included"),
        (omitted.pictures, "Pictures not included"),
        (omitted.figures, "Charts or diagrams shown by their alt text"),
    ] {
        if count > 0 {
            let _ = writeln!(out, "<li>{label}: {count}.</li>");
        }
    }
    out.push_str("</ul>\n</section>\n");
    out
}

/// The OPF package. Its many fields come straight from the OPC parts, so they arrive together in
/// `parts` and unpack here.
struct PackageParts<'a> {
    title: &'a str,
    author: &'a str,
    language: &'a str,
    identifier: &'a str,
    modified: &'a str,
    chapters: &'a [Chapter],
    images: &'a Images,
    meta: &'a model::Meta,
}

fn package_document(parts: &PackageParts<'_>) -> String {
    let PackageParts { title, author, language, identifier, modified, chapters, images, meta } = *parts;
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"{}\">\n",
        xml_attr(language)
    ));
    out.push_str("<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n");
    out.push_str(&format!("<dc:identifier id=\"bookid\">{}</dc:identifier>\n", xml_text(identifier)));
    out.push_str(&format!("<dc:title>{}</dc:title>\n", xml_text(title)));
    out.push_str(&format!("<dc:language>{}</dc:language>\n", xml_text(language)));
    if !author.is_empty() {
        out.push_str(&format!("<dc:creator>{}</dc:creator>\n", xml_text(author)));
    }
    if !meta.subject.trim().is_empty() {
        out.push_str(&format!("<dc:subject>{}</dc:subject>\n", xml_text(meta.subject.trim())));
    }
    if !meta.description.trim().is_empty() {
        out.push_str(&format!("<dc:description>{}</dc:description>\n", xml_text(meta.description.trim())));
    }
    for keyword in meta.keywords.split([',', ';']).map(str::trim).filter(|k| !k.is_empty()) {
        out.push_str(&format!("<dc:subject>{}</dc:subject>\n", xml_text(keyword)));
    }
    out.push_str(&format!("<meta property=\"dcterms:modified\">{}</meta>\n", xml_text(modified)));
    out.push_str("</metadata>\n<manifest>\n");
    out.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    out.push_str("<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n");
    for (i, chapter) in chapters.iter().enumerate() {
        let properties = if chapter.mathml { " properties=\"mathml\"" } else { "" };
        out.push_str(&format!(
            "<item id=\"chapter{}\" href=\"{}\" media-type=\"application/xhtml+xml\"{properties}/>\n",
            i + 1,
            xml_attr(&chapter.file)
        ));
    }
    for (i, image) in images.items.iter().enumerate() {
        out.push_str(&format!("<item id=\"image{}\" href=\"{}\" media-type=\"{}\"/>\n", i + 1, xml_attr(&image.name), xml_attr(image.media_type)));
    }
    out.push_str("</manifest>\n<spine>\n");
    for i in 0..chapters.len() {
        out.push_str(&format!("<itemref idref=\"chapter{}\"/>\n", i + 1));
    }
    out.push_str("</spine>\n</package>\n");
    out
}

fn nav_document(language: &str, title: &str, entries: &[NavEntry]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n");
    out.push_str(&format!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{}\" lang=\"{}\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>{}</title>\n</head>\n<body>\n<nav epub:type=\"toc\" id=\"toc\">\n<h1>{}</h1>\n<ol class=\"toc\">\n",
        xml_attr(language),
        xml_attr(language),
        xml_text(title),
        xml_text(title)
    ));
    for entry in entries {
        let href = if entry.anchor.is_empty() { entry.file.clone() } else { format!("{}#{}", entry.file, entry.anchor) };
        out.push_str(&format!(
            "<li class=\"toc-level-{}\"><a href=\"{}\">{}</a></li>\n",
            entry.level.clamp(1, 3),
            xml_attr(&href),
            xml_text(&entry.title)
        ));
    }
    out.push_str("</ol>\n</nav>\n</body>\n</html>\n");
    out
}

fn chapter_xhtml(language: &str, title: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{}\" lang=\"{}\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>{}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n</head>\n<body>\n{}</body>\n</html>\n",
        xml_attr(language),
        xml_attr(language),
        xml_text(title),
        body
    )
}

/// Escape text for XML content, dropping characters XML 1.0 cannot carry.
fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if valid_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// Escape an XML attribute value, dropping characters XML 1.0 cannot carry.
fn xml_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if valid_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

fn strip_invalid_xml(s: &str) -> String {
    s.chars().filter(|c| valid_xml_char(*c)).collect()
}

fn valid_xml_char(c: char) -> bool {
    matches!(c, '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

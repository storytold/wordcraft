//! WordCraft PDF export through the shared `craft-pdf` writer and Krilla drawing primitives.
//!
//! [`export`] lays the document out and draws each page's display list
//! ([`wordcraft_layout::display::page_display`]) into a PDF page:
//!
//! - **Text as real text:** glyph runs are written with embedded, subsetted fonts and a Unicode
//!   mapping taken from the paragraph text, so text is selectable, searchable and extractable.
//!   Synthetic bold (fill + stroke) and italic (skew) follow the renderer.
//! - **Graphics:** fills (highlight, shading, cell fills), rules and underlines (solid, dotted,
//!   dashed, double, wave), shapes, pictures (decoded with `image`; JPEG passed through, the rest
//!   embedded losslessly; WMF and EMF drawn as vector paths and embedded bitmaps) with cropping and
//!   opacity, the page colour and the watermark.
//! - **Interactive:** link annotations for hyperlinked runs (URLs and internal `#bookmark`
//!   links) and a document outline built from the headings (outline levels).
//! - **Metadata:** title, author, subject, keywords, language, creation date.
//! - **Tagged PDF (best effort):** one structure element per paragraph (`H1`–`H6` for headings,
//!   `P` otherwise), `Figure` for pictures with their alt text; headers, footers, decorations and
//!   the watermark are artifacts.
//!
//! Nothing here panics on hostile documents: sizes are clamped, unknown fonts or undecodable
//! pictures are skipped (a grey box stands in for a picture).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU16;
use std::sync::Arc;

use craft_pdf::write::{self, Metadata, WriteError, Writer, backend as krilla};
use krilla::action::{Action, LinkAction};
use krilla::annotation::{Annotation, LinkAnnotation, Target};
use krilla::color::rgb;
use krilla::destination::XyzDestination;
use krilla::error::KrillaError;
use krilla::geom::{Path, PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::metadata::DateTime;
use krilla::num::NormalizedF32;
use krilla::outline::{Outline, OutlineNode};
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::tagging::{Artifact, ArtifactType, ContentTag, Identifier, Node, SpanTag, Tag, TagGroup, TagKind, TagTree};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use wordcraft_doc::graphic::GraphicKind;
use wordcraft_doc::para::{InlineObject, ShapeKind};
use wordcraft_doc::{Document, Path as DocPath, Rgb, StoryRef};
use wordcraft_fonts::FaceRef;
use wordcraft_geom::Rect;
use wordcraft_layout::display::{DisplayOptions, Draw, Stroke as LineStyle, page_display};
use wordcraft_layout::wordart::ArtPaint;
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, Page, Placed, layout};
use wordcraft_metafile::{Picture, PlacedItem};

/// Largest page side we write (200", the PDF limit).
const MAX_SIDE: f32 = 14_400.0;
/// Largest picture side we embed (pixels).
const MAX_IMAGE_SIDE: u32 = 30_000;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PdfError {
    #[error("the document has no pages to export")]
    NoPages,
    #[error("page {0} does not exist")]
    BadPage(usize),
    #[error("PDF writing failed: {0}")]
    Write(String),
}

impl From<WriteError> for PdfError {
    fn from(error: WriteError) -> Self {
        Self::Write(error.to_string())
    }
}

/// Export options.
#[derive(Clone, Debug)]
pub struct PdfOptions {
    /// Pages to export (0-based layout page indexes, in output order); `None` = all.
    pub pages: Option<Vec<usize>>,
    /// Title / author overrides (default: the document properties).
    pub title: Option<String>,
    pub author: Option<String>,
    /// Write a structure tree (tagged PDF, best effort).
    pub tagged: bool,
    /// Show tracked changes as markup (coloured, underlined / struck through).
    pub include_markup: bool,
    /// Print Markup's comments: each page is scaled down to make room on its right for the
    /// comments in balloons joined to their anchors (Word's "Document showing markup").
    pub comments: bool,
    /// Compress content streams.
    pub compress: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        PdfOptions { pages: None, title: None, author: None, tagged: true, include_markup: false, comments: false, compress: true }
    }
}

/// Lay out `doc` and write it as PDF. Without markup the layout is the final text: tracked
/// deletions are left out, as in Word's "No Markup" view. With `comments` each page carries the
/// markup area with the comment balloons.
pub fn export(doc: &Document, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    write::guard(|| {
        let lo = LayoutOptions { hide_deleted: !opts.include_markup, comment_balloons: opts.comments, ..Default::default() };
        let lay = layout(doc, &mut LayoutCache::new(), &lo);
        export_layout_inner(doc, &lay, opts)
    })
}

/// Write an existing layout of `doc` as PDF.
pub fn export_layout(doc: &Document, lay: &DocLayout, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    write::guard(|| export_layout_inner(doc, lay, opts))
}

fn export_layout_inner(doc: &Document, lay: &DocLayout, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    let pages: Vec<usize> = match &opts.pages {
        Some(v) => {
            if let Some(bad) = v.iter().find(|p| **p >= lay.pages.len()) {
                return Err(PdfError::BadPage(bad.saturating_add(1)));
            }
            v.clone()
        }
        None => (0..lay.pages.len()).collect(),
    };
    if pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    // krilla subsets fonts only when the document is finished, so one font it can't subset fails
    // the whole export. Draw that font's glyphs as outlines instead and write the document again.
    let mut outlined = HashSet::new();
    loop {
        match write_attempt(doc, lay, opts, &pages, &outlined)? {
            Attempt::Done(bytes) => return Ok(bytes),
            Attempt::BadFont(id, msg) if outlined.len() < MAX_OUTLINED_FONTS && outlined.insert(id) => {
                log::warn!("PDF: {msg}; its text is drawn as outlines");
            }
            Attempt::BadFont(_, msg) => return Err(PdfError::Write(msg)),
        }
    }
}

/// How many unsubsettable fonts we fall back to outlines for before giving up.
const MAX_OUTLINED_FONTS: usize = 32;

/// One try at writing the PDF.
enum Attempt {
    Done(Vec<u8>),
    /// The face with this id could not be embedded (and why).
    BadFont(u32, String),
}

fn write_attempt(doc: &Document, lay: &DocLayout, opts: &PdfOptions, pages: &[usize], outlined: &HashSet<u32>) -> Result<Attempt, PdfError> {
    let settings = krilla::SerializeSettings { compress_content_streams: opts.compress, enable_tagging: opts.tagged, ..Default::default() };
    let language = doc.styles.default_chr.lang.clone().filter(|l| !l.is_empty() && l.is_ascii());
    let mut writer = Writer::new_with_language(settings, metadata(doc, opts), language);
    let pdf = writer.document_mut();

    let mut ex = Exporter {
        doc,
        lay,
        opts,
        out_index: pages.iter().enumerate().map(|(o, p)| (*p, o)).collect(),
        fonts: HashMap::new(),
        font_error: None,
        outlined,
        cmaps: HashMap::new(),
        images: HashMap::new(),
        vectors: HashMap::new(),
        tags: Vec::new(),
        tag_index: HashMap::new(),
        links: Vec::new(),
        bookmarks: std::cell::OnceCell::new(),
    };
    if let Some(o) = ex.outline(pages) {
        pdf.set_outline(o);
    }
    for &pi in pages {
        let Some(page) = lay.pages.get(pi) else { return Err(PdfError::BadPage(pi.saturating_add(1))) };
        let fit = Fit::of(page);
        let (w, h) = (clamp_side(fit.paper_w), clamp_side(page.h));
        let size = Size::from_wh(w, h).ok_or(PdfError::NoPages)?;
        let mut kp = pdf.start_page_with(PageSettings::new(size));
        let mut s = kp.surface();
        ex.links.clear();
        // A page with a markup area is scaled down to fit its paper with the area beside it.
        let scaled = fit.k < 1.0;
        if scaled {
            s.push_transform(&Transform::from_row(fit.k, 0.0, 0.0, fit.k, 0.0, fit.dy));
        }
        // `w` is the document page's width: the page colour and watermark stay off the markup area.
        ex.page(&mut s, page, w, h);
        if scaled {
            s.pop();
        }
        s.finish();
        for a in ex.annotations(fit) {
            kp.add_annotation(a);
        }
        kp.finish();
    }
    if opts.tagged {
        pdf.set_tag_tree(ex.tag_tree());
    }
    if let Some(error) = ex.font_error {
        return Err(error);
    }
    match writer.finish() {
        Ok(bytes) => Ok(Attempt::Done(bytes)),
        Err(WriteError::Serialization(error)) => match *error {
            KrillaError::Font(font, msg) => {
                let bad = ex.fonts.iter().find(|(_, (_, f))| f.as_ref() == Some(&font));
                match bad {
                    Some((id, (face, _))) => Ok(Attempt::BadFont(*id, format!("font {} {} could not be embedded ({msg})", face.family, face.style))),
                    None => Err(PdfError::Write(format!("a font could not be embedded ({msg})"))),
                }
            }
            error => Err(PdfError::Write(format!("{error:?}"))),
        },
        Err(error) => Err(error.into()),
    }
}

/// How a layout page maps onto its PDF page: a page with a markup area (Print Markup) is scaled
/// by `k` to its paper width and centred vertically (`dy`); any other page is drawn as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    paper_w: f32,
    k: f32,
    dy: f32,
}

impl Fit {
    fn of(page: &Page) -> Fit {
        let paper_w = page.w - page.markup;
        if page.markup > 0.0 && ok(page.markup) && ok(page.w) && paper_w > 1.0 {
            let k = paper_w / page.w;
            Fit { paper_w, k, dy: page.h * (1.0 - k) / 2.0 }
        } else {
            Fit { paper_w: page.w, k: 1.0, dy: 0.0 }
        }
    }

    /// A layout point on the PDF page.
    fn point(self, x: f32, y: f32) -> (f32, f32) {
        (x * self.k, y * self.k + self.dy)
    }
}

/// The outlines of `glyphs` (glyph id, pen position) at `size`, as one path in page coordinates.
fn glyph_outlines(face: &FaceRef, size: f32, glyphs: &[(u32, f32, f32)]) -> Option<Path> {
    let db = wordcraft_fonts::FontDb::global();
    let scale = f64::from(size) / face.upem.max(1.0);
    let mut pb = PathBuilder::new();
    for &(g, x, y) in glyphs {
        // Outlines are in font units, y-down: scale to the size, move to the pen position.
        let mut outline = (*db.outline(face, g)).clone();
        outline.apply_affine(kurbo::Affine::translate((f64::from(x), f64::from(y))) * kurbo::Affine::scale(scale));
        append_path(&mut pb, &outline);
    }
    pb.finish()
}

/// Whether a run's glyphs already run left to right, as PDF text extractors (PDFium in Chrome and
/// Edge, pdf.js, poppler) take them to: they read a run's glyphs as written left to right and turn
/// right-to-left words back into reading order themselves. Layout gives right-to-left text in
/// reading order.
fn left_to_right(glyphs: &[(u32, f32, f32)]) -> bool {
    glyphs.windows(2).all(|w| matches!(w, [a, b] if a.1 <= b.1))
}

/// Per-glyph text ranges for clusters of several glyphs (a letter and its vowel marks) when each
/// glyph is one of the cluster's characters (through the font's character map, one glyph at most
/// left to take the character left over), so a viewer
/// reversing a right-to-left word puts every mark back after its letter. Other clusters, such
/// as a ligature drawn as two glyphs, keep their shared range.
fn split_clusters(
    text: &str,
    glyphs: &[(u32, f32, f32)],
    ranges: &[std::ops::Range<usize>],
    rev: &HashMap<u32, char>,
) -> Vec<std::ops::Range<usize>> {
    let mut out = ranges.to_vec();
    let mut i = 0;
    while i < ranges.len() {
        let Some(r) = ranges.get(i) else { break };
        let n = ranges.get(i..).map_or(1, |rest| rest.iter().take_while(|x| *x == r).count()).max(1);
        let chars: Vec<(usize, char)> = text.get(r.clone()).map(|t| t.char_indices().map(|(k, c)| (r.start + k, c)).collect()).unwrap_or_default();
        if n > 1 && chars.len() == n {
            // Glyphs the character map knows take their characters; one left over (a letter's
            // contextual form, which the map doesn't list) takes the one character left.
            let mut used = vec![false; n];
            let mut of: Vec<Option<usize>> = (i..i + n)
                .map(|g| {
                    let c = rev.get(&glyphs.get(g)?.0)?;
                    let k = chars.iter().enumerate().position(|(k, (_, ch))| ch == c && !used.get(k).copied().unwrap_or(true))?;
                    *used.get_mut(k)? = true;
                    Some(k)
                })
                .collect();
            let free: Vec<usize> = (0..n).filter(|k| !used.get(*k).copied().unwrap_or(true)).collect();
            if let ([k], [slot]) = (free.as_slice(), of.iter_mut().filter(|o| o.is_none()).collect::<Vec<_>>().as_mut_slice()) {
                **slot = Some(*k);
            }
            let parts: Option<Vec<std::ops::Range<usize>>> = of
                .iter()
                .map(|k| {
                    let (a, ch) = chars.get((*k)?)?;
                    Some(*a..a + ch.len_utf8())
                })
                .collect();
            if let (Some(parts), Some(dst)) = (parts, out.get_mut(i..i + n)) {
                dst.clone_from_slice(&parts);
            }
        }
        i += n;
    }
    out
}

/// Indices of a run's glyphs in visual order, left to right: its clusters (consecutive glyphs
/// sharing a range in `clusters`, such as a letter and its marks, or a lam-alef ligature) sorted
/// by their leftmost glyph and kept together. Inside a cluster, glyphs go in reverse order of
/// their own text (`parts`) when it is right to left, so a viewer reversing the word reads the
/// letter before its marks.
fn visual_order(text: &str, glyphs: &[(u32, f32, f32)], clusters: &[std::ops::Range<usize>], parts: &[std::ops::Range<usize>]) -> Vec<usize> {
    let mut groups: Vec<(f32, Vec<usize>)> = Vec::new();
    for (i, (_, x, _)) in glyphs.iter().enumerate() {
        match groups.last_mut() {
            Some((left, idx)) if i > 0 && clusters.get(i).is_some() && clusters.get(i) == clusters.get(i - 1) => {
                *left = left.min(*x);
                idx.push(i);
            }
            _ => groups.push((*x, vec![i])),
        }
    }
    // Stable: clusters at the same x keep their order, as do glyphs sharing their text.
    groups.sort_by(|a, b| a.0.total_cmp(&b.0));
    groups
        .into_iter()
        .flat_map(|(_, mut idx)| {
            let rtl = idx.first().and_then(|&g| text.get(clusters.get(g)?.clone())).is_some_and(|t| t.chars().any(wordcraft_fonts::is_rtl));
            if rtl {
                idx.sort_by_key(|&g| std::cmp::Reverse(parts.get(g).map_or(0, |r| r.start)));
            }
            idx
        })
        .collect()
}

/// The y most of a run's glyphs share: its baseline, which raised or lowered marks leave.
fn baseline(glyphs: &[(u32, f32, f32)]) -> Option<f32> {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for (_, _, y) in glyphs {
        *counts.entry(y.to_bits()).or_insert(0) += 1;
    }
    // Ties go to the lower y bits, so the result doesn't depend on hash order.
    counts.into_iter().max_by_key(|&(bits, n)| (n, std::cmp::Reverse(bits))).map(|(bits, _)| f32::from_bits(bits))
}

/// A chart's or diagram's alt text: its own, else what it is ("chart", "diagram").
fn graphic_alt(alt: &str, kind: GraphicKind) -> &str {
    if alt.trim().is_empty() { kind.noun() } else { alt }
}

/// Add the segments of a kurbo path to a krilla path.
fn append_path(pb: &mut PathBuilder, path: &kurbo::BezPath) {
    let f = |v: f64| v as f32;
    for el in path.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => pb.move_to(f(p.x), f(p.y)),
            kurbo::PathEl::LineTo(p) => pb.line_to(f(p.x), f(p.y)),
            kurbo::PathEl::QuadTo(a, p) => pb.quad_to(f(a.x), f(a.y), f(p.x), f(p.y)),
            kurbo::PathEl::CurveTo(a, b, p) => pb.cubic_to(f(a.x), f(a.y), f(b.x), f(b.y), f(p.x), f(p.y)),
            kurbo::PathEl::ClosePath => pb.close(),
        }
    }
}

fn clamp_side(v: f32) -> f32 {
    if v.is_finite() { v.clamp(3.0, MAX_SIDE) } else { 612.0 }
}

fn metadata(doc: &Document, opts: &PdfOptions) -> Metadata {
    let c = &doc.core;
    let mut m = Metadata { creator: "WordCraft".into(), producer: "WordCraft".into(), ..Default::default() };
    let title = opts.title.clone().unwrap_or_else(|| c.title.clone());
    if !title.trim().is_empty() {
        m.title = Some(title);
    }
    let author = opts.author.clone().unwrap_or_else(|| c.creator.clone());
    if !author.trim().is_empty() {
        m.authors = vec![author];
    }
    if !c.subject.trim().is_empty() {
        m.description = Some(c.subject.clone());
    } else if !c.description.trim().is_empty() {
        m.description = Some(c.description.clone());
    }
    let kw: Vec<String> = c.keywords.split([',', ';']).map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
    if !kw.is_empty() {
        m.keywords = kw;
    }
    let date = parse_iso(&c.created).or_else(|| now_unix().map(civil));
    if let Some((y, mo, d, h, mi, s)) = date {
        m.created =
            Some(DateTime::new(y.clamp(0, 9999) as u16).month(mo).day(d).hour(h).minute(mi).second(s).utc_offset_hour(0).utc_offset_minute(0));
    }
    m
}

type Civil = (i64, u8, u8, u8, u8, u8);

/// `YYYY-MM-DD[THH:MM[:SS]]…` → civil time.
fn parse_iso(s: &str) -> Option<Civil> {
    let s = s.trim();
    let n = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let h = n(11, 13).unwrap_or(0).clamp(0, 23);
    let mi = n(14, 16).unwrap_or(0).clamp(0, 59);
    let sec = n(17, 19).unwrap_or(0).clamp(0, 59);
    Some((y, mo as u8, d as u8, h as u8, mi as u8, sec as u8))
}

/// The system clock, or the browser's on the web.
fn now_unix() -> Option<i64> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        i64::try_from(secs).ok()
    }
    // `SystemTime::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(target_arch = "wasm32")]
    {
        let ms = js_sys::Date::now();
        (ms.is_finite() && ms > 0.0).then(|| (ms / 1000.0) as i64)
    }
}

/// Unix seconds (UTC) → civil time (proleptic Gregorian).
fn civil(t: i64) -> Civil {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u8, day as u8, (secs / 3600) as u8, (secs / 60 % 60) as u8, (secs % 60) as u8)
}

fn norm(v: f32) -> NormalizedF32 {
    NormalizedF32::new(if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }).unwrap_or(NormalizedF32::ONE)
}

fn fill(c: Rgb, alpha: f32) -> Fill {
    Fill { paint: rgb::Color::new(c.0, c.1, c.2).into(), opacity: norm(alpha), rule: FillRule::NonZero }
}

fn ok(v: f32) -> bool {
    v.is_finite() && v.abs() < 1e6
}

fn rect_path(r: &Rect) -> Option<Path> {
    if !(ok(r.x) && ok(r.y) && ok(r.w) && ok(r.h)) || r.w <= 0.0 || r.h <= 0.0 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.push_rect(krilla::geom::Rect::from_xywh(r.x, r.y, r.w, r.h)?);
    pb.finish()
}

/// `r` grown by `g` points on every side and moved by (`dx`, `dy`).
fn grown(r: &Rect, g: f32, dx: f32, dy: f32) -> Rect {
    Rect { x: r.x - g + dx, y: r.y - g + dy, w: r.w + 2.0 * g, h: r.h + 2.0 * g }
}

/// A shape's shadow and glow, behind it: bands of its silhouette (the same approximation of a
/// blur as the raster renderer, see [`wordcraft_doc::effects::bands`]).
fn shape_effects(
    s: &mut Surface,
    kind: ShapeKind,
    corner: Option<u32>,
    rect: &Rect,
    fx: &wordcraft_doc::effects::ShapeEffects,
    filled: bool,
    stroked: bool,
    sw: f32,
) {
    let sw = if sw.is_finite() { sw.clamp(0.25, 200.0) } else { 0.75 };
    let silhouette = |s: &mut Surface, c: Rgb, a: f32, g: f32, dx: f32, dy: f32| {
        if filled {
            let grow = g + if stroked { sw / 2.0 } else { 0.0 };
            if let Some(p) = shape_path(kind, &grown(rect, grow, dx, dy), corner) {
                s.set_stroke(None);
                s.set_fill(Some(fill(c, a)));
                s.draw_path(&p);
            }
        } else if sw + 2.0 * g > 0.05
            && let Some(p) = shape_path(kind, &grown(rect, 0.0, dx, dy), corner)
        {
            s.set_fill(None);
            s.set_stroke(Some(Stroke { paint: rgb::Color::new(c.0, c.1, c.2).into(), width: sw + 2.0 * g, opacity: norm(a), ..Default::default() }));
            s.draw_path(&p);
        }
    };
    if let Some(sh) = fx.shadow {
        let (dx, dy) = sh.offset();
        for (g, a) in wordcraft_doc::effects::bands(-sh.blur / 2.0, sh.blur / 2.0, sh.opacity()) {
            silhouette(s, sh.color, a, g, dx, dy);
        }
    }
    if let Some(gl) = fx.glow {
        for (g, a) in wordcraft_doc::effects::bands(0.0, gl.size, gl.opacity()) {
            silhouette(s, gl.color, a, g, 0.0, 0.0);
        }
    }
    s.set_fill(None);
    s.set_stroke(None);
}

/// Outline of a basic shape in a rectangle (same geometry as the raster renderer); `corner` is a
/// rounded rectangle's ([`wordcraft_doc::connector::ShapeExtra::corner`]).
fn shape_path(kind: ShapeKind, r: &Rect, corner: Option<u32>) -> Option<Path> {
    let flat_ok = kind.is_open() && r.w.max(r.h) > 0.0 && r.w.min(r.h) >= 0.0;
    if !(ok(r.x) && ok(r.y) && ok(r.w) && ok(r.h)) || !(flat_ok || r.w > 0.0 && r.h > 0.0) {
        return None;
    }
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let (cx, cy) = (x0 + r.w / 2.0, y0 + r.h / 2.0);
    let mut pb = PathBuilder::new();
    let poly = |pb: &mut PathBuilder, pts: &[(f32, f32)]| {
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(*x, *y);
            } else {
                pb.line_to(*x, *y);
            }
        }
        pb.close();
    };
    const K: f32 = 0.552_284_8;
    let ellipse = |pb: &mut PathBuilder, cx: f32, cy: f32, rx: f32, ry: f32| {
        pb.move_to(cx + rx, cy);
        pb.cubic_to(cx + rx, cy + ry * K, cx + rx * K, cy + ry, cx, cy + ry);
        pb.cubic_to(cx - rx * K, cy + ry, cx - rx, cy + ry * K, cx - rx, cy);
        pb.cubic_to(cx - rx, cy - ry * K, cx - rx * K, cy - ry, cx, cy - ry);
        pb.cubic_to(cx + rx * K, cy - ry, cx + rx, cy - ry * K, cx + rx, cy);
        pb.close();
    };
    match kind {
        // A freeform is drawn from its own paths; without them, its frame.
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::Freeform => poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]),
        ShapeKind::RoundedRectangle => {
            let rad = wordcraft_doc::connector::corner_radius(corner, r.w, r.h);
            let k = rad * (1.0 - K);
            pb.move_to(x0 + rad, y0);
            pb.line_to(x1 - rad, y0);
            pb.cubic_to(x1 - k, y0, x1, y0 + k, x1, y0 + rad);
            pb.line_to(x1, y1 - rad);
            pb.cubic_to(x1, y1 - k, x1 - k, y1, x1 - rad, y1);
            pb.line_to(x0 + rad, y1);
            pb.cubic_to(x0 + k, y1, x0, y1 - k, x0, y1 - rad);
            pb.line_to(x0, y0 + rad);
            pb.cubic_to(x0, y0 + k, x0 + k, y0, x0 + rad, y0);
            pb.close();
        }
        ShapeKind::Ellipse => ellipse(&mut pb, cx, cy, r.w / 2.0, r.h / 2.0),
        ShapeKind::FlowchartOr => {
            ellipse(&mut pb, cx, cy, r.w / 2.0, r.h / 2.0);
            pb.move_to(x0, cy);
            pb.line_to(x1, cy);
            pb.move_to(cx, y0);
            pb.line_to(cx, y1);
        }
        ShapeKind::Triangle => poly(&mut pb, &[(cx, y0), (x1, y1), (x0, y1)]),
        ShapeKind::RightTriangle => poly(&mut pb, &[(x0, y0), (x1, y1), (x0, y1)]),
        ShapeKind::Diamond => poly(&mut pb, &[(cx, y0), (x1, cy), (cx, y1), (x0, cy)]),
        // DrawingML's `line`: top-left to bottom-right (flips give the other directions).
        ShapeKind::Line => {
            pb.move_to(x0, y0);
            pb.line_to(x1, y1);
        }
        // Connectors are drawn as paths (`Draw::Path`); this is their frame's diagonal.
        ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => {
            pb.move_to(x0, y0);
            pb.line_to(x1, y1);
        }
        ShapeKind::Arrow => {
            let h = r.h;
            poly(
                &mut pb,
                &[
                    (x0, cy - h * 0.2),
                    (x1 - h * 0.5, cy - h * 0.2),
                    (x1 - h * 0.5, y0),
                    (x1, cy),
                    (x1 - h * 0.5, y1),
                    (x1 - h * 0.5, cy + h * 0.2),
                    (x0, cy + h * 0.2),
                ],
            )
        }
        ShapeKind::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = std::f32::consts::PI * i as f32 / 5.0 - std::f32::consts::FRAC_PI_2;
                    let rad = if i % 2 == 0 { 1.0 } else { 0.4 };
                    (cx + a.cos() * r.w / 2.0 * rad, cy + a.sin() * r.h / 2.0 * rad)
                })
                .collect();
            poly(&mut pb, &pts);
        }
        ShapeKind::Heart => {
            let (w, h) = (r.w, r.h);
            pb.move_to(cx, y0 + h * 0.3);
            pb.cubic_to(cx, y0, x0, y0, x0, y0 + h * 0.3);
            pb.cubic_to(x0, y0 + h * 0.6, cx - w * 0.1, y0 + h * 0.75, cx, y1);
            pb.cubic_to(cx + w * 0.1, y0 + h * 0.75, x1, y0 + h * 0.6, x1, y0 + h * 0.3);
            pb.cubic_to(x1, y0, cx, y0, cx, y0 + h * 0.3);
            pb.close();
        }
    }
    pb.finish()
}

/// A structure element being collected (one per paragraph, in reading order).
struct TagEntry {
    kind: TagKind,
    children: Vec<Node>,
}

struct Exporter<'a> {
    doc: &'a Document,
    lay: &'a DocLayout,
    opts: &'a PdfOptions,
    /// Layout page index → output page index.
    out_index: HashMap<usize, usize>,
    /// Embedded fonts by face id (`None`: krilla can't read it).
    fonts: HashMap<u32, (FaceRef, Option<Font>)>,
    font_error: Option<PdfError>,
    /// Faces whose glyphs are drawn as outlines because krilla can't subset them.
    outlined: &'a HashSet<u32>,
    cmaps: HashMap<u32, Arc<HashMap<u32, char>>>,
    images: HashMap<String, Option<Image>>,
    /// Parsed metafiles by media key (`None`: not readable).
    vectors: HashMap<String, Option<Arc<Vector>>>,
    tags: Vec<TagEntry>,
    tag_index: HashMap<(StoryRef, DocPath), usize>,
    /// Link rectangles on the current page: (x0, y0, x1, y1, target).
    links: Vec<(f32, f32, f32, f32, String)>,
    /// Bookmark positions by name (the first of a name wins), gathered once: looking each internal
    /// link up by walking the whole document made a long document with a linked table of contents
    /// take seconds to export.
    bookmarks: std::cell::OnceCell<HashMap<String, wordcraft_doc::Pos>>,
}

/// What the content being drawn is, for tagging.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    /// Paragraph content of the structure entry at this index.
    Para(usize),
    /// Header/footer, decorations: not part of the structure.
    Artifact(ArtifactType),
    /// A floating picture (its own Figure).
    Figure,
}

impl Exporter<'_> {
    fn font(&mut self, face: &FaceRef) -> Option<Font> {
        if self.outlined.contains(&face.id()) {
            return None;
        }
        let font_error = &mut self.font_error;
        self.fonts
            .entry(face.id())
            .or_insert_with(|| {
                let variations = face.is_variable().then_some(face.coords.as_slice());
                let font = match write::font(face.data(), face.index(), variations) {
                    Ok(font) => Some(font),
                    // Keep WordCraft's existing policy: unreadable faces are skipped.
                    Err(WriteError::InvalidFont) => None,
                    Err(error) => {
                        // Allocation, validation or backend failures must fail the export,
                        // rather than return a PDF with missing text.
                        font_error.get_or_insert_with(|| error.into());
                        None
                    }
                };
                (*face, font)
            })
            .1
            .clone()
    }

    fn cmap(&mut self, face: &FaceRef) -> Arc<HashMap<u32, char>> {
        self.cmaps
            .entry(face.id())
            .or_insert_with(|| {
                let mut m = HashMap::new();
                for (c, g) in face.chars() {
                    m.entry(g).or_insert(c);
                }
                Arc::new(m)
            })
            .clone()
    }

    /// The metafile picture for a media key, parsed once. None when the bytes are not a metafile
    /// (the raster path takes them) or do not parse (the raster path then draws the grey box).
    fn vector(&mut self, key: &str) -> Option<Arc<Vector>> {
        let doc = self.doc;
        let bytes = doc.media.get(key)?;
        if !wordcraft_metafile::is_metafile(bytes) {
            return None;
        }
        if let Some(v) = self.vectors.get(key) {
            return v.clone();
        }
        let v = build_vector(bytes).map(Arc::new);
        if v.is_none() {
            log::warn!("PDF: metafile `{key}` could not be read");
        }
        self.vectors.insert(key.to_string(), v.clone());
        v
    }

    fn image(&mut self, key: &str) -> Option<Image> {
        if let Some(i) = self.images.get(key) {
            return i.clone();
        }
        let img = self.doc.media.get(key).and_then(load_image);
        if img.is_none() {
            log::warn!("PDF: picture `{key}` could not be decoded");
        }
        self.images.insert(key.to_string(), img.clone());
        img
    }

    /// Run `f` inside a marked-content section for `role`.
    fn tagged(&mut self, s: &mut Surface, role: Role, alt: Option<&str>, f: impl FnOnce(&mut Self, &mut Surface)) {
        if !self.opts.tagged {
            f(self, s);
            return;
        }
        match role {
            Role::Artifact(kind) => {
                s.start_tagged(ContentTag::Artifact(Artifact::new(kind, None)));
                f(self, s);
                s.end_tagged();
            }
            Role::Para(i) => {
                let id = s.start_tagged(ContentTag::Span(SpanTag::empty()));
                f(self, s);
                s.end_tagged();
                if let Some(e) = self.tags.get_mut(i) {
                    e.children.push(Node::Leaf(id));
                }
            }
            Role::Figure => {
                let id: Identifier = s.start_tagged(ContentTag::Other);
                f(self, s);
                s.end_tagged();
                let alt = alt.map(str::to_string).filter(|a| !a.is_empty()).or_else(|| Some("picture".to_string()));
                let g = TagGroup::with_children(Tag::Figure(alt), vec![Node::Leaf(id)]);
                self.tags.push(TagEntry { kind: Tag::Div.into(), children: vec![Node::Group(g)] });
            }
        }
    }

    fn page(&mut self, s: &mut Surface, page: &Page, w: f32, h: f32) {
        if let Some(c) = self.doc.settings.page_color
            && let Some(p) = rect_path(&Rect::new(0.0, 0.0, w, h))
        {
            self.tagged(s, Role::Artifact(ArtifactType::Other), None, |_, s| {
                s.set_stroke(None);
                s.set_fill(Some(fill(c, 1.0)));
                s.draw_path(&p);
                s.set_fill(None);
            });
        }
        if let Some(pic) = self.doc.settings.page_picture.clone() {
            self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| me.page_picture(s, &pic, w, h));
        }
        if let Some(wm) = self.doc.settings.watermark.clone()
            && !wm.text.trim().is_empty()
        {
            self.tagged(s, Role::Artifact(ArtifactType::Watermark), None, |me, s| me.watermark(s, &wm, w, h));
        }
        let dopts = DisplayOptions {
            marks: false,
            dim_header: false,
            dim_body: false,
            markup: self.opts.include_markup,
            placeholders: false,
            ..Default::default()
        };
        for it in page.header.iter().chain(page.footer.iter()) {
            let draws = self.draws(page, it, &dopts);
            self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| {
                for d in &draws {
                    me.draw(s, d);
                }
            });
        }
        for it in &page.items {
            let draws = self.draws(page, it, &dopts);
            match it {
                Placed::Lines { story, path, para, .. } => {
                    let idx = self.entry(*story, path, para);
                    for d in &draws {
                        match d {
                            Draw::Glyphs { .. } | Draw::Turned { .. } => self.tagged(s, Role::Para(idx), None, |me, s| me.draw(s, d)),
                            Draw::Image { alt, .. } => self.tagged(s, Role::Figure, Some(alt), |me, s| me.draw(s, d)),
                            // A rotated inline picture.
                            Draw::Rotated { items, .. } if items.iter().any(|i| matches!(i, Draw::Image { .. })) => {
                                let alt = items.iter().find_map(|i| if let Draw::Image { alt, .. } = i { Some(alt.as_str()) } else { None });
                                self.tagged(s, Role::Figure, alt, |me, s| me.draw(s, d))
                            }
                            // An inline chart or diagram: its text is part of the figure, not the paragraph.
                            Draw::Figure { alt, kind, draws: inner } => self.tagged(s, Role::Figure, Some(graphic_alt(alt, *kind)), |me, s| {
                                for d in inner {
                                    me.draw(s, d);
                                }
                            }),
                            _ => self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| me.draw(s, d)),
                        }
                    }
                }
                Placed::Image { story, path, off, .. } => {
                    let alt = match self.doc.para(*story, path).and_then(|p| p.object_at(*off)) {
                        Some(InlineObject::Image { alt, .. }) => alt.clone(),
                        _ => String::new(),
                    };
                    self.tagged(s, Role::Figure, Some(&alt), |me, s| {
                        for d in &draws {
                            me.draw(s, d);
                        }
                    });
                }
                Placed::Graphic { story, path, off, graphic, .. } => {
                    let alt = match self.doc.para(*story, path).and_then(|p| p.object_at(*off)) {
                        Some(InlineObject::Graphic { alt, .. }) => graphic_alt(alt, graphic.kind).to_string(),
                        _ => graphic.kind.noun().to_string(),
                    };
                    self.tagged(s, Role::Figure, Some(&alt), |me, s| {
                        for d in &draws {
                            me.draw(s, d);
                        }
                    });
                }
                _ => self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| {
                    for d in &draws {
                        me.draw(s, d);
                    }
                }),
            }
        }
    }

    /// Display items for one placed item.
    fn draws(&self, page: &Page, it: &Placed, dopts: &DisplayOptions) -> Vec<Draw> {
        let mut items = vec![it.clone()];
        // Lines in a cell cut off at its edge are drawn inside its clip: `page_display` finds the
        // cell among the items (cells themselves draw nothing).
        if matches!(it, Placed::Lines { .. }) {
            let clipped = page.header.iter().chain(&page.footer).chain(&page.items).filter(|c| matches!(c, Placed::Cell { clip: true, .. }));
            items.extend(clipped.cloned());
        }
        let one = Page { w: page.w, h: page.h, section: page.section, number: page.number, items, body: page.body, ..Default::default() };
        page_display(self.doc, &one, dopts)
    }

    /// A heading's text for bookmarks and tags: what the pages show. That leaves out what its
    /// layout `pl` left out (hidden text, resolved as laid out, so table formatting counts too) and,
    /// without markup, tracked deletions.
    fn title_text(&self, p: &wordcraft_doc::Paragraph, pl: &wordcraft_layout::para::ParaLayout) -> String {
        let mut dropped = pl.left_out.clone();
        if !self.opts.include_markup {
            dropped.extend(p.deleted_ranges());
        }
        p.text_without(&dropped)
    }

    /// The structure entry for a paragraph (created on first sight, in reading order).
    fn entry(&mut self, story: StoryRef, path: &DocPath, pl: &wordcraft_layout::para::ParaLayout) -> usize {
        if let Some(i) = self.tag_index.get(&(story, path.clone())) {
            return *i;
        }
        let kind: TagKind = match self.doc.para(story, path).map(|p| self.doc.styles.resolve_para(&p.props).outline_level) {
            Some(Some(l)) if l < 6 => {
                let title = self.doc.para(story, path).map(|p| self.title_text(p, pl).trim().chars().take(200).collect::<String>());
                Tag::Hn(NonZeroU16::new(u16::from(l) + 1).unwrap_or(NonZeroU16::MIN), title).into()
            }
            _ => Tag::P.into(),
        };
        self.tags.push(TagEntry { kind, children: Vec::new() });
        let i = self.tags.len() - 1;
        self.tag_index.insert((story, path.clone()), i);
        i
    }

    fn tag_tree(&mut self) -> TagTree {
        let mut tree = TagTree::new().with_lang(self.doc.styles.default_chr.lang.clone());
        for e in std::mem::take(&mut self.tags) {
            if e.children.is_empty() {
                continue;
            }
            tree.push(TagGroup::with_children(e.kind, e.children));
        }
        tree
    }

    fn draw(&mut self, s: &mut Surface, d: &Draw) {
        match d {
            Draw::Figure { draws, .. } => {
                for d in draws {
                    self.draw(s, d);
                }
            }
            Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges, .. } => {
                self.glyphs(s, face, *size, glyphs, *color, *alpha, *synth_bold, *synth_italic, text, ranges);
                if let Some(l) = link {
                    self.link_rect(face, *size, glyphs, l);
                }
            }
            Draw::Fill { rect, color, alpha, .. } => {
                if let Some(p) = rect_path(rect) {
                    s.set_stroke(None);
                    s.set_fill(Some(fill(*color, *alpha)));
                    s.draw_path(&p);
                    s.set_fill(None);
                }
            }
            Draw::Line { x0, y0, x1, y1, width, color, stroke, alpha, .. } => self.line(s, (*x0, *y0, *x1, *y1), *width, *color, *stroke, *alpha),
            Draw::Image { rect, media, crop, alpha, .. } => self.picture(s, rect, media, crop, *alpha),
            Draw::Path { segs, fill: f, stroke, stroke_width } => {
                let mut pb = PathBuilder::new();
                append_path(&mut pb, &wordcraft_layout::display::seg_path(segs));
                let Some(p) = pb.finish() else { return };
                s.set_fill(f.map(|c| fill(c, 1.0)));
                s.set_stroke(stroke.map(|c| Stroke {
                    paint: rgb::Color::new(c.0, c.1, c.2).into(),
                    // Width 0 (or none) is a hairline.
                    width: if stroke_width.is_finite() && *stroke_width > 0.0 { stroke_width.clamp(0.25, 200.0) } else { 0.75 },
                    ..Default::default()
                }));
                if f.is_some() || stroke.is_some() {
                    s.draw_path(&p);
                }
                s.set_fill(None);
                s.set_stroke(None);
            }
            Draw::Ink { pts, color, width, alpha } => {
                let mut pb = PathBuilder::new();
                let mut used = 0usize;
                let mut last = (0.0, 0.0);
                for &(x, y) in pts.iter().filter(|(x, y)| ok(*x) && ok(*y)) {
                    if used > 0 {
                        pb.line_to(x, y);
                    } else {
                        pb.move_to(x, y);
                    }
                    used += 1;
                    last = (x, y);
                }
                // A tap (one usable point): a zero-length line, which round caps draw as a dot.
                if used == 1 {
                    pb.line_to(last.0, last.1);
                }
                let Some(p) = pb.finish() else { return };
                s.set_fill(None);
                s.set_stroke(Some(Stroke {
                    paint: rgb::Color::new(color.0, color.1, color.2).into(),
                    width: if width.is_finite() { width.clamp(0.25, 200.0) } else { 1.0 },
                    opacity: norm(*alpha),
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Default::default()
                }));
                s.draw_path(&p);
                s.set_stroke(None);
            }
            Draw::Shape { rect, kind, fill: f, stroke, stroke_width, effects, corner } => {
                let Some(p) = shape_path(*kind, rect, *corner) else { return };
                let can_fill = *kind != ShapeKind::Line;
                let fx = effects.sanitized();
                if !fx.is_empty() && (f.is_some() && can_fill || stroke.is_some()) {
                    shape_effects(s, *kind, *corner, rect, &fx, f.is_some() && can_fill, stroke.is_some(), *stroke_width);
                }
                // Soft edges: the fill fades out toward the outline (which fades with it).
                if let (Some(rad), Some(c), true) = (fx.soft_edge, f, can_fill) {
                    let rad = rad.min(rect.w.min(rect.h) / 2.0).max(0.0);
                    s.set_stroke(None);
                    for (g, a) in wordcraft_doc::effects::bands(-rad, 0.0, 1.0) {
                        if let Some(p) = shape_path(*kind, &grown(rect, g, 0.0, 0.0), *corner) {
                            s.set_fill(Some(fill(*c, a)));
                            s.draw_path(&p);
                        }
                    }
                    s.set_fill(None);
                    return;
                }
                s.set_fill(f.filter(|_| can_fill).map(|c| fill(c, 1.0)));
                s.set_stroke(stroke.map(|c| Stroke {
                    paint: rgb::Color::new(c.0, c.1, c.2).into(),
                    width: if stroke_width.is_finite() { stroke_width.clamp(0.25, 200.0) } else { 0.75 },
                    ..Default::default()
                }));
                if f.is_some() && can_fill || stroke.is_some() {
                    s.draw_path(&p);
                }
                s.set_fill(None);
                s.set_stroke(None);
            }
            Draw::Art { path, fx, color, alpha } => {
                for l in wordcraft_layout::wordart::art_layers(path, fx, *color, *alpha) {
                    let mut pb = PathBuilder::new();
                    append_path(&mut pb, &l.path);
                    let Some(p) = pb.finish() else { continue };
                    let (paint, opacity): (krilla::paint::Paint, f32) = match &l.paint {
                        ArtPaint::Solid(c, a) => (rgb::Color::new(c.0, c.1, c.2).into(), *a),
                        ArtPaint::Linear { p0, p1, stops } => {
                            let stops = stops
                                .iter()
                                .map(|(o, c, a)| krilla::paint::Stop {
                                    offset: norm(*o),
                                    color: rgb::Color::new(c.0, c.1, c.2).into(),
                                    opacity: norm(*a),
                                })
                                .collect();
                            let g = krilla::paint::LinearGradient {
                                x1: p0.0,
                                y1: p0.1,
                                x2: p1.0,
                                y2: p1.1,
                                transform: Transform::default(),
                                spread_method: krilla::paint::SpreadMethod::Pad,
                                stops,
                                anti_alias: true,
                            };
                            (g.into(), 1.0)
                        }
                    };
                    match l.stroke {
                        Some(w) => {
                            s.set_fill(None);
                            s.set_stroke(Some(Stroke {
                                paint,
                                width: w.clamp(0.1, 400.0),
                                opacity: norm(opacity),
                                line_join: LineJoin::Round,
                                ..Default::default()
                            }));
                        }
                        None => {
                            s.set_stroke(None);
                            s.set_fill(Some(Fill { paint, opacity: norm(opacity), rule: FillRule::NonZero }));
                        }
                    }
                    s.draw_path(&p);
                }
                s.set_fill(None);
                s.set_stroke(None);
            }
            Draw::Mark { .. } | Draw::MarkText { .. } => {}
            Draw::Clip { rect, items } => {
                let Some(clip) = rect_path(rect) else { return };
                s.push_clip_path(&clip, &FillRule::NonZero);
                for it in items {
                    self.draw(s, it);
                }
                s.pop();
            }
            Draw::Turned { x, y, turn, items } => {
                if !ok(*x) || !ok(*y) {
                    return;
                }
                let [a, b, c, d, e, f] = Draw::turn_matrix(*turn, *x, *y);
                s.push_transform(&Transform::from_row(a, b, c, d, e, f));
                for it in items {
                    self.draw(s, it);
                }
                s.pop();
            }
            Draw::Rotated { cx, cy, spin, items } => {
                if !ok(*cx) || !ok(*cy) {
                    return;
                }
                let [a, b, c, d, e, f] = spin.matrix(*cx, *cy);
                s.push_transform(&Transform::from_row(a, b, c, d, e, f));
                for it in items {
                    self.draw(s, it);
                }
                s.pop();
            }
        }
    }

    fn line(&mut self, s: &mut Surface, (x0, y0, x1, y1): (f32, f32, f32, f32), width: f32, c: Rgb, style: LineStyle, alpha: f32) {
        if ![x0, y0, x1, y1].iter().all(|v| ok(*v)) {
            return;
        }
        let w = if width.is_finite() { width.clamp(0.1, 100.0) } else { 0.5 };
        let paint: krilla::paint::Paint = rgb::Color::new(c.0, c.1, c.2).into();
        let stroke = |width: f32, dash: Option<StrokeDash>, cap: LineCap| Stroke {
            paint: paint.clone(),
            width,
            opacity: norm(alpha),
            dash,
            line_cap: cap,
            ..Default::default()
        };
        let seg = |s: &mut Surface, dx: f32, dy: f32| {
            let mut pb = PathBuilder::new();
            pb.move_to(x0 + dx, y0 + dy);
            pb.line_to(x1 + dx, y1 + dy);
            if let Some(p) = pb.finish() {
                s.draw_path(&p);
            }
        };
        s.set_fill(None);
        match style {
            LineStyle::Solid => {
                s.set_stroke(Some(stroke(w, None, LineCap::Butt)));
                seg(s, 0.0, 0.0);
            }
            LineStyle::Dotted => {
                s.set_stroke(Some(stroke(w, Some(StrokeDash { array: vec![w, w * 2.0], offset: 0.0 }), LineCap::Butt)));
                seg(s, 0.0, 0.0);
            }
            LineStyle::Dashed => {
                s.set_stroke(Some(stroke(w, Some(StrokeDash { array: vec![w * 4.0, w * 2.0], offset: 0.0 }), LineCap::Butt)));
                seg(s, 0.0, 0.0);
            }
            LineStyle::Double => {
                // Two thinner lines either side of the centre line.
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                let (nx, ny) = (-dy / len * w * 0.9, dx / len * w * 0.9);
                s.set_stroke(Some(stroke(w * 0.6, None, LineCap::Butt)));
                seg(s, nx, ny);
                seg(s, -nx, -ny);
            }
            LineStyle::Wave => {
                let (a, b) = (x0.min(x1), x0.max(x1));
                let amp = (w * 1.2).max(0.8);
                let step = amp * 2.0;
                let mut pb = PathBuilder::new();
                pb.move_to(a, y0);
                let mut x = a;
                let mut up = true;
                while x < b && x - a < 20_000.0 {
                    let nx = (x + step).min(b);
                    pb.quad_to((x + nx) / 2.0, if up { y0 - amp } else { y0 + amp }, nx, y0);
                    x = nx;
                    up = !up;
                }
                s.set_stroke(Some(stroke(w * 0.8, None, LineCap::Round)));
                if let Some(p) = pb.finish() {
                    s.draw_path(&p);
                }
            }
        }
        s.set_stroke(None);
    }

    /// The page's picture or texture fill: tiled at 96 dpi for a `tile` fill (when that is a sane
    /// number of tiles), else stretched over the page.
    fn page_picture(&mut self, s: &mut Surface, pic: &wordcraft_doc::PagePicture, w: f32, h: f32) {
        const MAX_TILES: f32 = 4096.0;
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
            return;
        }
        let dims = self
            .doc
            .media
            .get(&pic.media)
            .and_then(|b| image::ImageReader::new(std::io::Cursor::new(b.as_slice())).with_guessed_format().ok()?.into_dimensions().ok());
        if let Some((pw, ph)) = dims.filter(|_| pic.fill_type.eq_ignore_ascii_case("tile")) {
            let (tw, th) = (pw as f32 * 0.75, ph as f32 * 0.75);
            if tw >= 1.0 && th >= 1.0 && (w / tw).ceil() * (h / th).ceil() <= MAX_TILES {
                let mut y = 0.0;
                while y < h {
                    let mut x = 0.0;
                    while x < w {
                        self.picture(s, &Rect::new(x, y, tw, th), &pic.media, &[0.0; 4], 1.0);
                        x += tw;
                    }
                    y += th;
                }
                return;
            }
        }
        self.picture(s, &Rect::new(0.0, 0.0, w, h), &pic.media, &[0.0; 4], 1.0);
    }

    fn picture(&mut self, s: &mut Surface, rect: &Rect, media: &str, crop: &[f32; 4], alpha: f32) {
        let Some(clip) = rect_path(rect) else { return };
        if let Some(v) = self.vector(media) {
            vector_picture(s, &v, rect, crop, &clip, alpha);
            return;
        }
        let Some(img) = self.image(media) else {
            s.set_stroke(None);
            s.set_fill(Some(fill(Rgb(0xD0, 0xD0, 0xD0), 1.0)));
            s.draw_path(&clip);
            s.set_fill(None);
            return;
        };
        let [cl, ct, cr, cb] = crop.map(|v| if v.is_finite() { v.clamp(0.0, 0.95) } else { 0.0 });
        let vis_w = (1.0 - cl - cr).max(0.05);
        let vis_h = (1.0 - ct - cb).max(0.05);
        let (fw, fh) = (rect.w / vis_w, rect.h / vis_h);
        let Some(size) = Size::from_wh(fw.max(0.01), fh.max(0.01)) else { return };
        let fade = alpha < 0.999;
        if fade {
            s.push_opacity(norm(alpha));
        }
        s.push_clip_path(&clip, &FillRule::NonZero);
        s.push_transform(&Transform::from_translate(rect.x - cl * fw, rect.y - ct * fh));
        s.draw_image(img, size);
        s.pop();
        s.pop();
        if fade {
            s.pop();
        }
    }

    /// Unicode text for each glyph: (text, per-glyph byte range). Glyphs and the run's text are
    /// aligned through the font's reverse character map (ligatures take several characters,
    /// extra glyphs share their cluster's text).
    fn map_text(&mut self, face: &FaceRef, glyphs: &[(u32, f32, f32)], text: &str) -> (String, Vec<std::ops::Range<usize>>) {
        let cmap = self.cmap(face);
        let rev = |g: u32| cmap.get(&g).copied();
        if text.is_empty() {
            let mut t = String::new();
            let ranges = glyphs
                .iter()
                .map(|(g, _, _)| {
                    let a = t.len();
                    t.push(rev(*g).unwrap_or(' '));
                    a..t.len()
                })
                .collect();
            return (t, ranges);
        }
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let n = chars.len();
        let byte = |i: usize| chars.get(i).map(|c| c.0).unwrap_or(text.len());
        let mut ranges = Vec::with_capacity(glyphs.len());
        let mut i = 0usize;
        for (k, (g, _, _)) in glyphs.iter().enumerate() {
            let left_g = glyphs.len() - k;
            let left_c = n.saturating_sub(i);
            if left_c == 0 {
                let r = ranges.last().cloned().unwrap_or(0..text.len());
                ranges.push(r);
                continue;
            }
            if k + 1 == glyphs.len() {
                ranges.push(byte(i)..text.len());
                i = n;
                continue;
            }
            let this = chars.get(i).map(|c| c.1);
            let take = if left_c <= left_g || rev(*g) == this {
                if left_g > left_c && rev(*g) != this && k > 0 {
                    // More glyphs than characters: this one shares the previous cluster.
                    let r = ranges.last().cloned().unwrap_or(byte(i)..byte(i + 1));
                    ranges.push(r);
                    continue;
                }
                1
            } else {
                // A ligature (or substituted glyph): consume until the next glyph's character.
                let next = glyphs.get(k + 1).and_then(|(ng, _, _)| rev(*ng));
                let max = left_c - (left_g - 1);
                let mut t = 1;
                while t < max && chars.get(i + t).map(|c| c.1) != next {
                    t += 1;
                }
                if t == max && next.is_some() && chars.get(i + t).map(|c| c.1) != next {
                    // The next glyph doesn't match anything: assume one character each.
                    t = 1;
                }
                t
            };
            ranges.push(byte(i)..byte(i + take));
            i += take;
        }
        (text.to_string(), ranges)
    }

    #[allow(clippy::too_many_arguments)]
    fn glyphs(
        &mut self,
        s: &mut Surface,
        face: &FaceRef,
        size: f32,
        glyphs: &[(u32, f32, f32)],
        color: Rgb,
        alpha: f32,
        bold: bool,
        italic: bool,
        text: &str,
        known: &[std::ops::Range<usize>],
    ) {
        if !size.is_finite() || size <= 0.0 || size > 5000.0 {
            return;
        }
        if glyphs.is_empty() {
            return;
        }
        if !glyphs.iter().all(|(_, x, y)| ok(*x) && ok(*y)) {
            return;
        }
        let font = self.font(face);
        if font.is_none() && !self.outlined.contains(&face.id()) {
            log::warn!("PDF: font {} {} could not be embedded", face.family, face.style);
            return;
        }
        // Layout's glyph → text mapping when it has one (right-to-left text, contextual forms and
        // ligatures can't be guessed back from the font's cmap), else a guess.
        let valid = known.len() == glyphs.len() && known.iter().all(|r| r.start < r.end && text.get(r.clone()).is_some());
        let (txt, ranges) = if valid { (text.to_string(), known.to_vec()) } else { self.map_text(face, glyphs, text) };
        let upem = face.upem.max(1.0) as f32;
        // Right-to-left text is written left to right as it appears (see `left_to_right`); glyphs
        // already in that order are written as they are.
        let (order, ranges) = if left_to_right(glyphs) {
            ((0..glyphs.len()).collect(), ranges)
        } else {
            let parts = split_clusters(&txt, glyphs, &ranges, &self.cmap(face));
            (visual_order(&txt, glyphs, &ranges, &parts), parts)
        };
        let placed: Vec<(u32, f32, f32, std::ops::Range<usize>)> =
            order.iter().filter_map(|&i| Some((glyphs.get(i).copied()?, ranges.get(i).cloned()?))).map(|((g, x, y), r)| (g, x, y, r)).collect();
        let Some(&(_, x0, y0, _)) = placed.first() else { return };
        // The run's baseline: the y most of its glyphs share (marks sit higher or lower).
        let base = baseline(glyphs).unwrap_or(y0);
        let kg: Vec<KrillaGlyph> = placed
            .iter()
            .enumerate()
            .map(|(k, (g, x, y, r))| {
                let adv = match placed.get(k + 1) {
                    Some((_, nx, _, _)) => (nx - x) / size,
                    None => face.advance(*g) as f32 / upem,
                };
                KrillaGlyph::new(GlyphId::new(*g), adv, 0.0, (y0 - y) / size, 0.0, r.clone(), None)
            })
            .collect();
        let pushed = italic;
        if italic {
            s.push_transform(&Transform::from_row(1.0, 0.0, -0.21, 1.0, 0.21 * base, 0.0));
        }
        s.set_fill(Some(fill(color, alpha)));
        s.set_stroke(if bold {
            Some(Stroke { paint: rgb::Color::new(color.0, color.1, color.2).into(), width: size * 0.03, opacity: norm(alpha), ..Default::default() })
        } else {
            None
        });
        match font {
            Some(font) => {
                s.draw_glyphs(Point::from_xy(x0, y0), &kg, font, &txt, size, false);
            }
            None => {
                if let Some(path) = glyph_outlines(face, size, glyphs) {
                    s.draw_path(&path);
                }
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
        if pushed {
            s.pop();
        }
    }

    fn link_rect(&mut self, face: &FaceRef, size: f32, glyphs: &[(u32, f32, f32)], link: &str) {
        let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else { return };
        let adv = face.advance(last.0) as f32 * size / face.upem.max(1.0) as f32;
        let (x0, x1) = (first.1.min(last.1), first.1.max(last.1) + adv);
        let (y0, y1) = (first.2 - size * 0.85, first.2 + size * 0.25);
        if ![x0, x1, y0, y1].iter().all(|v| ok(*v)) {
            return;
        }
        if let Some(prev) = self.links.last_mut()
            && prev.4 == link
            && (prev.1 - y0).abs() < 0.5
            && x0 - prev.2 < size
        {
            prev.0 = prev.0.min(x0);
            prev.2 = prev.2.max(x1);
            return;
        }
        if self.links.len() < 10_000 {
            self.links.push((x0, y0, x1, y1, link.to_string()));
        }
    }

    /// Where a bookmark is: (output page, x, y) — the page and line its start lands on, which
    /// in a paragraph split across pages need not be the paragraph's first page.
    fn bookmark(&self, name: &str) -> Option<(usize, f32, f32)> {
        let marks = self.bookmarks.get_or_init(|| {
            let mut m = HashMap::new();
            for (n, pos) in self.doc.bookmarks() {
                m.entry(n).or_insert(pos);
            }
            m
        });
        let pos = marks
            .get(name)
            .or_else(|| marks.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))?
            .clone();
        if let Some(c) = self.lay.caret(&pos)
            && let Some(out) = self.out_index.get(&c.page)
        {
            // The left edge of the paragraph's piece on that page, at the top of the line.
            let x = self
                .lay
                .index
                .get(&(pos.story, pos.path.clone()))
                .into_iter()
                .flatten()
                .filter(|(pi, _)| *pi == c.page)
                .find_map(|(pi, ii)| match self.lay.pages.get(*pi).and_then(|p| p.items.get(*ii)) {
                    Some(Placed::Lines { x, .. }) => Some(*x),
                    _ => None,
                })
                .unwrap_or(c.x);
            let (x, y) = self
                .lay
                .pages
                .get(c.page)
                .map(|p| Fit::of(p).point(x, c.top))
                .unwrap_or((x, c.top));
            return Some((*out, x, y));
        }
        self.locate(pos.story, &pos.path)
    }

    /// The first output page showing a paragraph, with the top of its first line.
    fn locate(&self, story: StoryRef, path: &DocPath) -> Option<(usize, f32, f32)> {
        let places = self.lay.index.get(&(story, path.clone()))?;
        for (pi, ii) in places {
            let Some(out) = self.out_index.get(pi) else { continue };
            let Some(page) = self.lay.pages.get(*pi) else { continue };
            let (x, y) = match page.items.get(*ii) {
                Some(Placed::Lines { x, y, .. }) => Fit::of(page).point(*x, *y),
                _ => (0.0, 0.0),
            };
            return Some((*out, x, y));
        }
        None
    }

    fn annotations(&self, fit: Fit) -> Vec<Annotation> {
        let mut v = Vec::new();
        for (x0, y0, x1, y1, target) in &self.links {
            let ((x0, y0), (x1, y1)) = (fit.point(*x0, *y0), fit.point(*x1, *y1));
            let Some(rect) = krilla::geom::Rect::from_ltrb(x0, y0, x1, y1) else { continue };
            let t = if let Some(name) = target.strip_prefix('#') {
                match self.bookmark(name) {
                    Some((page, x, y)) => Target::Destination(XyzDestination::new(page, Point::from_xy(x, y)).into()),
                    None => continue,
                }
            } else {
                let url = target.trim();
                if url.is_empty() {
                    continue;
                }
                Target::Action(Action::Link(LinkAction::new(url.to_string())))
            };
            v.push(Annotation::new_link(LinkAnnotation::new(rect, t), Some(target.clone())));
        }
        v
    }

    /// The outline from body headings (outline levels 1–9) on the exported pages.
    fn outline(&self, pages: &[usize]) -> Option<Outline> {
        let mut flat: Vec<(u8, String, usize, f32, f32)> = Vec::new();
        for (out, pi) in pages.iter().enumerate() {
            let Some(page) = self.lay.pages.get(*pi) else { continue };
            for it in &page.items {
                let Placed::Lines { story: StoryRef::Body, path, para, l0: 0, x, y, .. } = it else { continue };
                let Some(p) = self.doc.para(StoryRef::Body, path) else { continue };
                let Some(level) = self.doc.styles.resolve_para(&p.props).outline_level else { continue };
                let title: String = self.title_text(p, para).split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
                if title.is_empty() {
                    continue;
                }
                let (x, y) = Fit::of(page).point(*x, *y);
                flat.push((level.min(8), title, out, x, y));
            }
        }
        if flat.is_empty() {
            return None;
        }
        fn build(flat: &[(u8, String, usize, f32, f32)], i: &mut usize, level: u8) -> Vec<OutlineNode> {
            let mut out = Vec::new();
            while let Some((l, title, page, x, y)) = flat.get(*i) {
                if *l < level {
                    break;
                }
                *i += 1;
                let mut node = OutlineNode::new(title.clone(), XyzDestination::new(*page, Point::from_xy(*x, *y)));
                // Children: following entries with a deeper level.
                if flat.get(*i).is_some_and(|n| n.0 > *l) {
                    for c in build(flat, i, l + 1) {
                        node.push_child(c);
                    }
                }
                out.push(node);
            }
            out
        }
        let mut o = Outline::new();
        let mut i = 0;
        while i < flat.len() {
            let before = i;
            for n in build(&flat, &mut i, 0) {
                o.push_child(n);
            }
            if i == before {
                i += 1;
            }
        }
        Some(o)
    }

    fn watermark(&mut self, s: &mut Surface, wm: &wordcraft_doc::Watermark, w: f32, h: f32) {
        let r = wordcraft_fonts::word::resolve(&wm.font, false, false);
        let face = r.face;
        let text: String = wm.text.chars().take(200).collect();
        let shaped = wordcraft_fonts::shape(&face, &text, &[], |c| c);
        let upem = face.upem.max(1.0) as f32;
        let raw_w: f32 = shaped.iter().map(|g| g.x_advance as f32).sum::<f32>() / upem;
        if raw_w <= 0.0 || !raw_w.is_finite() {
            return;
        }
        let hh = h.min(2000.0);
        let diag = if wm.diagonal { (w * w + hh * hh).sqrt() } else { w };
        let size = (diag * 0.7 / raw_w).min(300.0);
        let angle = if wm.diagonal { -(hh / w.max(1.0)).atan() } else { 0.0 };
        let (c, sn) = (angle.cos(), angle.sin());
        let (dx, dy) = (-raw_w * size / 2.0, size * 0.35);
        let (cx, cy) = (w / 2.0, hh / 2.0);
        let font = self.font(&face);
        if font.is_none() && !self.outlined.contains(&face.id()) {
            return;
        }
        let glyphs: Vec<KrillaGlyph> = shaped
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let end = shaped.iter().skip(i + 1).map(|n| n.cluster).find(|cl| *cl > g.cluster).unwrap_or(text.len());
                let start = g.cluster.min(end);
                KrillaGlyph::new(
                    GlyphId::new(g.gid),
                    g.x_advance as f32 / upem,
                    g.x_offset as f32 / upem,
                    g.y_offset as f32 / upem,
                    0.0,
                    start..end,
                    None,
                )
            })
            .collect();
        s.push_transform(&Transform::from_row(c, sn, -sn, c, c * dx - sn * dy + cx, sn * dx + c * dy + cy));
        s.set_stroke(None);
        s.set_fill(Some(fill(wm.color, if wm.semitransparent { 0.5 } else { 1.0 })));
        match font {
            Some(font) => {
                s.draw_glyphs(Point::from_xy(0.0, 0.0), &glyphs, font, &text, size, false);
            }
            None => {
                let k = size / upem;
                let mut x = 0.0;
                let mut pen = Vec::with_capacity(shaped.len());
                for g in &shaped {
                    pen.push((g.gid, x + g.x_offset as f32 * k, -(g.y_offset as f32) * k));
                    x += g.x_advance as f32 * k;
                }
                if let Some(path) = glyph_outlines(&face, size, &pen) {
                    s.draw_path(&path);
                }
            }
        }
        s.set_fill(None);
        s.pop();
    }
}

/// A metafile picture, parsed once. The geometry comes placed by the metafile crate; the bitmaps are
/// embedded images, one per picture item (None for paths and undecodable bitmaps).
struct Vector {
    pic: Picture,
    images: Vec<Option<Image>>,
}

/// Parses a metafile and embeds its bitmaps. None when the picture has no usable size.
fn build_vector(bytes: &[u8]) -> Option<Vector> {
    let mut pic = wordcraft_metafile::parse(bytes).ok()?;
    if !pic.has_size() {
        return None;
    }
    let images = pic.items.iter_mut().map(|it| it.take_pixels().map(|(w, h, px)| Image::from_rgba8(px, w, h))).collect();
    Some(Vector { pic, images })
}

/// A straight RGBA colour as a krilla colour and opacity.
fn paint_of(c: [u8; 4]) -> (rgb::Color, NormalizedF32) {
    (rgb::Color::new(c[0], c[1], c[2]), norm(f32::from(c[3]) / 255.0))
}

/// Narrowest metafile stroke in points. Cosmetic (width 0) pens get it rather than PDF's 0-width
/// "thinnest line": krilla bounds a 0-width stroke by the bare path, which is empty for a horizontal
/// line, so a faded (grouped) picture would clip it away.
const HAIRLINE: f64 = 0.25;

/// Draws a metafile picture into the clip (the picture frame): paths are filled and stroked in page
/// space (strokes keep their width under non-uniform scaling), bitmaps stretched, all faded by `alpha`.
/// Every path sets its fill and stroke, so none carries over; strokes have round caps and joins.
fn vector_picture(s: &mut Surface, v: &Vector, r: &Rect, crop: &[f32; 4], clip: &Path, alpha: f32) {
    let target = kurbo::Rect::new(f64::from(r.x), f64::from(r.y), f64::from(r.x + r.w), f64::from(r.y + r.h));
    let fade = alpha < 0.999;
    if fade {
        s.push_opacity(norm(alpha));
    }
    s.push_clip_path(clip, &FillRule::NonZero);
    for it in v.pic.place(target, *crop) {
        match it {
            PlacedItem::Path { path, fill, stroke, even_odd } => {
                let Some(p) = path_from(&path) else { continue };
                let fill = fill.map(|c| {
                    let (paint, opacity) = paint_of(c);
                    let rule = if even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
                    Fill { paint: paint.into(), opacity, rule }
                });
                let stroke = stroke.map(|(c, w)| {
                    let (paint, opacity) = paint_of(c);
                    Stroke {
                        paint: paint.into(),
                        width: w.max(HAIRLINE) as f32,
                        opacity,
                        line_cap: LineCap::Round,
                        line_join: LineJoin::Round,
                        ..Default::default()
                    }
                });
                s.set_fill(fill);
                s.set_stroke(stroke);
                s.draw_path(&p);
            }
            PlacedItem::Bitmap { index, rect } => {
                let Some(Some(img)) = v.images.get(index) else { continue };
                let Some(size) = Size::from_wh(rect.width() as f32, rect.height() as f32) else { continue };
                s.push_transform(&Transform::from_translate(rect.x0 as f32, rect.y0 as f32));
                s.draw_image(img.clone(), size);
                s.pop();
            }
        }
    }
    s.set_fill(None);
    s.set_stroke(None);
    s.pop();
    if fade {
        s.pop();
    }
}

/// A kurbo path as a krilla path (None when empty).
fn path_from(b: &kurbo::BezPath) -> Option<Path> {
    let mut pb = PathBuilder::new();
    append_path(&mut pb, b);
    pb.finish()
}

/// Decode a picture for embedding (validated with the `image` crate first, so a broken file
/// can't fail the whole export).
fn load_image(data: &Arc<Vec<u8>>) -> Option<Image> {
    let fmt = image::guess_format(data).ok()?;
    // The side check below used to run only after a full decode; applying it as a decode limit
    // rejects a small file that declares a huge image at header time instead. The default
    // allocation cap (512 MB, what `load_from_memory_with_format` applied) stays in force.
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data.as_slice())).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    reader.limits(limits);
    let dec = reader.decode().ok()?;
    let (w, h) = (dec.width(), dec.height());
    if w == 0 || h == 0 || w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        return None;
    }
    if fmt == image::ImageFormat::Jpeg && matches!(dec.color(), image::ColorType::L8 | image::ColorType::Rgb8) {
        return Image::from_jpeg(data.clone().into(), true).ok();
    }
    Some(Image::from_rgba8(dec.to_rgba8().into_raw(), w, h))
}

#[cfg(test)]
mod tests;

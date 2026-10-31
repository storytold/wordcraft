use super::export::Fit;
use super::glyphs::{baseline, cluster_ends, glyph_id, glyph_outlines, left_to_right, split_clusters, visual_order};
use super::options::{PdfError, PdfOptions};
use super::paint::{append_path, fill, grown, norm, ok, paint_fill, pdf_line_cap, rect_path, shape_effects, shape_path};
use super::picture::{Vector, build_vector, load_image, paint_of, vector_picture};
use super::tags::{Role, TagEntry, figure_alt, graphic_alt, holds_picture};
use super::{
    Annotation, Arc, Artifact, ArtifactType, ClKind, ContentTag, DisplayOptions, DocLayout, DocPath, Document, Draw, FaceRef, Fill, FillRule, Font,
    HashMap, HashSet, Identifier, Image, InlineObject, ItemDisplay, KrillaGlyph, LineCap, LineEnd, LineJoin, LineStyle, LinkAnnotation, Node,
    NonZeroU16, Outline, OutlineNode, Page, PathBuilder, Placed, Point, Rect, Rgb, ShapeKind, Size, SpanTag, StoryRef, Stroke, StrokeDash, Surface,
    Tag, TagGroup, TagKind, TagTree, Target, Transform, WriteError, XyzDestination, krilla, rgb, write,
};
use wordcraft_doc::picfx::PicFx;

mod draw;
mod nav;
mod text;

pub(crate) struct Exporter<'a> {
    pub(crate) doc: &'a Document,
    pub(crate) lay: &'a DocLayout,
    pub(crate) opts: &'a PdfOptions,
    /// Layout page index → output page index.
    pub(crate) out_index: HashMap<usize, usize>,
    /// Embedded fonts by face id (`None`: krilla can't read it).
    pub(crate) fonts: HashMap<u32, (FaceRef, Option<Font>)>,
    pub(crate) font_error: Option<PdfError>,
    /// Faces whose glyphs are drawn as outlines because krilla can't subset them.
    pub(crate) outlined: &'a HashSet<u32>,
    pub(crate) cmaps: HashMap<u32, Arc<HashMap<u32, char>>>,
    /// Embedded pictures by media key and the brightness and contrast baked into them, so one
    /// picture with the same corrections is embedded once however often it's drawn.
    pub(crate) images: HashMap<(String, i32, i32), Option<Image>>,
    /// Parsed metafiles by media key (`None`: not readable).
    pub(crate) vectors: HashMap<String, Option<Arc<Vector>>>,
    pub(crate) tags: Vec<TagEntry>,
    pub(crate) tag_index: HashMap<(StoryRef, DocPath), usize>,
    /// Link rectangles on the current page: (x0, y0, x1, y1, target).
    pub(crate) links: Vec<(f32, f32, f32, f32, String)>,
    /// Bookmark positions by name (the first of a name wins), gathered once: looking each internal
    /// link up by walking the whole document made a long document with a linked table of contents
    /// take seconds to export.
    /// Bookmarks by exact name, and by ASCII-lowercased name for the case-blind fallback; the
    /// first in document order wins either way.
    pub(crate) bookmarks: std::cell::OnceCell<(HashMap<String, wordcraft_doc::Pos>, HashMap<String, wordcraft_doc::Pos>)>,
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

    fn image(&mut self, key: &str, fx: PicFx) -> Option<Image> {
        // Corrected pixels are embedded apart from the picture's own (#1711, #1766).
        let fx = fx.clamped();
        let cache_key = (key.to_string(), fx.bright, fx.contrast);
        if let Some(i) = self.images.get(&cache_key) {
            return i.clone();
        }
        let img = self.doc.media.get(key).and_then(|d| load_image(d, fx));
        if img.is_none() {
            log::warn!("PDF: picture `{key}` could not be decoded");
        }
        self.images.insert(cache_key, img.clone());
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

    /// A rotated inline picture or group: turned as a whole, with each picture in it tagged on its
    /// own, so a decorative picture is an artifact while the others stay figures with their alt
    /// text. A described shape's figure keeps its description; anything else is an artifact.
    fn rotated_pictures(&mut self, s: &mut Surface, d: &Draw) {
        match d {
            Draw::Rotated { cx, cy, spin, items } => {
                if !ok(*cx) || !ok(*cy) {
                    return;
                }
                let [a, b, c, d, e, f] = spin.matrix(*cx, *cy);
                s.push_transform(&Transform::from_row(a, b, c, d, e, f));
                for it in items {
                    self.rotated_pictures(s, it);
                }
                s.pop();
            }
            Draw::Image { decorative: true, .. } => self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| me.draw(s, d)),
            Draw::Image { alt, .. } => self.tagged(s, Role::Figure, Some(alt), |me, s| me.draw(s, d)),
            Draw::Figure { alt, draws, .. } => self.tagged(s, Role::Figure, Some(alt), |me, s| {
                for d in draws {
                    me.draw(s, d);
                }
            }),
            _ => self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| me.draw(s, d)),
        }
    }

    pub(crate) fn page(&mut self, s: &mut Surface, page: &Page, w: f32, h: f32) {
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
            let area = page.body;
            self.tagged(s, Role::Artifact(ArtifactType::Watermark), None, |me, s| me.watermark(s, &wm, w, h, area));
        }
        let dopts = DisplayOptions {
            marks: false,
            dim_header: false,
            dim_body: false,
            markup: self.opts.include_markup,
            placeholders: false,
            ..Default::default()
        };
        // What the items share (the authors' colours, the clipped cells) is worked out once per page.
        let shown = ItemDisplay::new(self.doc, page);
        for it in page.header.iter().chain(page.footer.iter()) {
            let draws = shown.draws(it, &dopts);
            self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| {
                for d in &draws {
                    me.draw(s, d);
                }
            });
        }
        for it in &page.items {
            let draws = shown.draws(it, &dopts);
            match it {
                Placed::Lines { story, path, para, .. } => {
                    let idx = self.entry(*story, path, para);
                    for d in &draws {
                        match d {
                            Draw::Glyphs { .. } | Draw::Turned { .. } => self.tagged(s, Role::Para(idx), None, |me, s| me.draw(s, d)),
                            // A picture marked decorative: an artifact screen readers skip.
                            Draw::Image { decorative: true, .. } => self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| me.draw(s, d)),
                            Draw::Image { alt, .. } => self.tagged(s, Role::Figure, Some(alt), |me, s| me.draw(s, d)),
                            // A rotated inline picture, or a rotated group holding pictures: turned as a
                            // whole, each picture tagged on its own (see `rotated_pictures`).
                            Draw::Rotated { items, .. } if items.iter().any(holds_picture) => self.rotated_pictures(s, d),
                            // An inline chart, diagram or described shape: its text is part of the figure, not the paragraph.
                            Draw::Figure { alt, kind, draws: inner } => self.tagged(s, Role::Figure, Some(figure_alt(alt, *kind)), |me, s| {
                                for d in inner {
                                    me.draw(s, d);
                                }
                            }),
                            _ => self.tagged(s, Role::Artifact(ArtifactType::Other), None, |me, s| me.draw(s, d)),
                        }
                    }
                }
                Placed::Image { story, path, off, .. } => {
                    let obj = self.doc.para(*story, path).and_then(|p| p.object_at(*off));
                    let role = if obj.is_some_and(InlineObject::is_decorative) { Role::Artifact(ArtifactType::Other) } else { Role::Figure };
                    let alt = match obj {
                        Some(InlineObject::Image { alt, .. }) => alt.clone(),
                        _ => String::new(),
                    };
                    self.tagged(s, role, Some(&alt), |me, s| {
                        for d in &draws {
                            me.draw(s, d);
                        }
                    });
                }
                // A floating shape with a description: a figure (see `shape_figure_alt`).
                Placed::Shape { alt, .. } if !alt.is_empty() => self.tagged(s, Role::Figure, Some(alt), |me, s| {
                    for d in &draws {
                        me.draw(s, d);
                    }
                }),
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

    pub(crate) fn tag_tree(&mut self) -> TagTree {
        let mut tree = TagTree::new().with_lang(self.doc.styles.default_chr.lang.clone());
        for e in std::mem::take(&mut self.tags) {
            if e.children.is_empty() {
                continue;
            }
            tree.push(TagGroup::with_children(e.kind, e.children));
        }
        tree
    }

    /// The watermark text stretched over its box (Word's `v:textpath fitshape`): its advance across
    /// the box's width, its em down the box's height with the em's foot (the font's descent below
    /// the baseline) on the box's bottom; the box centred on the margins (`area`, else the page)
    /// and, when diagonal, turned 45° counterclockwise.
    fn watermark(&mut self, s: &mut Surface, wm: &wordcraft_doc::Watermark, w: f32, h: f32, area: Rect) {
        let r = wordcraft_fonts::word::resolve(&wm.font, false, false);
        let face = r.face;
        let text: String = wm.text.chars().take(200).collect();
        let shaped = wordcraft_fonts::shape(&face, &text, &[], |c| c);
        let upem = face.upem.max(1.0) as f32;
        let raw_w: f32 = shaped.iter().map(|g| g.x_advance as f32).sum::<f32>() / upem;
        if raw_w <= 0.0 || !raw_w.is_finite() {
            return;
        }
        let [bw, bh] = wm.frame(w, h);
        let size = bh.min(2000.0);
        let stretch = bw / (raw_w * size);
        let descent = (wordcraft_fonts::word::line_metrics(&face).1 / face.upem.max(1.0)) as f32;
        let angle = if wm.diagonal { -std::f32::consts::FRAC_PI_4 } else { 0.0 };
        let (c, sn) = (angle.cos(), angle.sin());
        let (dx, dy) = (-bw / 2.0, bh / 2.0 - descent.clamp(0.0, 1.0) * bh);
        let (cx, cy) = if area.w > 0.0 && area.h > 0.0 { (area.x + area.w / 2.0, area.y + area.h / 2.0) } else { (w / 2.0, h.min(2000.0) / 2.0) };
        let font = self.font(&face);
        if font.is_none() && !self.outlined.contains(&face.id()) {
            return;
        }
        let ends = cluster_ends(&shaped.iter().map(|g| g.cluster).collect::<Vec<_>>(), text.len());
        let glyphs: Vec<KrillaGlyph> = shaped
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let end = ends.get(i).copied().unwrap_or(text.len());
                let start = g.cluster.min(end);
                KrillaGlyph::new(
                    glyph_id(g.gid),
                    g.x_advance as f32 / upem,
                    g.x_offset as f32 / upem,
                    g.y_offset as f32 / upem,
                    0.0,
                    start..end,
                    None,
                )
            })
            .collect();
        s.push_transform(&Transform::from_row(c * stretch, sn * stretch, -sn, c, c * dx - sn * dy + cx, sn * dx + c * dy + cy));
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

//! Rendered previews: tiny documents laid out and rasterised by the real engine and handed to the
//! framework's `Picture` as images (style, style-set and table-style tiles, equations, glyphs and
//! the Manage Styles sample), plus the font list the font boxes share.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::theme::Mode;
use craft::ui::display::Picture;
use craft::ui::list::ThumbAspect;
use craft::ui::primitives::textures::{LoadedPicture, Revision, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::runtime::env::Label;
use egui::{Color32, ColorImage, Vec2};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Paragraph, Table, para_block};

use crate::WordApp;

/// The font list and the style-sheet fingerprint the previews are keyed by.
#[derive(Default)]
pub struct Previews {
    /// The font list, for the interface language it was made in.
    families: Option<(&'static str, Vec<String>)>,
    font_gen: u64,
    /// Lowercase English family → its name in that language (simsun → 宋体), and back.
    local_names: HashMap<String, String>,
    english_names: HashMap<String, String>,
    /// The Chinese, Japanese and Korean fonts of the font list (the Asian text font box).
    east_asian: Option<Vec<String>>,
    styles_hash: (u64, u64),
}

impl Previews {
    /// Ensure the cached family lists match the current font generation.
    fn check_fonts(&mut self) {
        let generation = wordcraft_fonts::FontDb::global().generation();
        if self.font_gen != generation {
            self.fonts_changed();
        }
    }

    /// The font list: installed families, each by its name in the interface language when it has
    /// one (宋体 for SimSun in Chinese, #505), like Word's.
    pub fn families(&mut self) -> Vec<String> {
        self.check_fonts();
        let lang = crate::i18n::current().code();
        if self.families.as_ref().is_none_or(|(l, _)| *l != lang) {
            let db = wordcraft_fonts::FontDb::global();
            let mut f: Vec<String> = db.families_in(lang).into_iter().filter(|f| !f.starts_with('.')).collect();
            for w in [wordcraft_doc::styles::BODY_FONT, wordcraft_doc::styles::HEADING_FONT] {
                if !f.iter().any(|x| x == w) {
                    f.push(w.to_string());
                }
            }
            f.sort_by_key(|a| a.to_lowercase());
            f.dedup();
            let names = db.localized_names(lang);
            self.local_names = names.iter().map(|(e, l)| (e.to_lowercase(), l.clone())).collect();
            self.english_names = names.into_iter().map(|(e, l)| (l, e)).collect();
            self.families = Some((lang, f));
            self.east_asian = None;
        }
        self.families.as_ref().map(|(_, f)| f.clone()).unwrap_or_default()
    }

    /// The Font dialog's Asian text font list: the installed Chinese, Japanese and Korean fonts,
    /// named as in [`Previews::families`].
    pub fn east_asian_families(&mut self) -> Vec<String> {
        self.families();
        if self.east_asian.is_none() {
            let mut f: Vec<String> = wordcraft_fonts::FontDb::global()
                .east_asian_families()
                .into_iter()
                .filter(|f| !f.starts_with('.'))
                .map(|f| self.local_names.get(&f.to_lowercase()).cloned().unwrap_or(f))
                .collect();
            f.sort_by_key(|a| a.to_lowercase());
            f.dedup();
            self.east_asian = Some(f);
        }
        self.east_asian.clone().unwrap_or_default()
    }

    /// The name the font box shows for the document's font `family`: its name in the interface
    /// language when the font has one.
    pub fn font_shown(&mut self, family: &str) -> String {
        self.families();
        self.local_names.get(&family.to_lowercase()).cloned().unwrap_or_else(|| family.to_string())
    }

    /// The family name a font chosen in the font box is stored as: the English name for a font
    /// listed by its localized name (宋体 → SimSun), so the document reads the same anywhere.
    pub fn font_stored(&mut self, shown: &str) -> String {
        self.families();
        self.english_names.get(shown).cloned().unwrap_or_else(|| shown.to_string())
    }

    /// Fonts were added or removed: list the families again. Pictures see the change through the
    /// font generation in their revision ([`preview_revision`]).
    pub fn fonts_changed(&mut self) {
        self.families = None;
        self.east_asian = None;
        self.font_gen = wordcraft_fonts::FontDb::global().generation();
    }

    /// Another document is shown: its revisions count apart from the last one's.
    pub(crate) fn document_changed(&mut self) {
        self.styles_hash = (u64::MAX, 0);
    }

    /// Hash of the style sheet (recomputed only when the document revision changes).
    // lean: the document has no styles generation, so the sheet is serialised and hashed on each
    // revision; a generation counter on the style sheet would replace this.
    pub(crate) fn styles_key(&mut self, app_doc: &Document, rev: u64) -> u64 {
        if self.styles_hash.0 != rev {
            let s = serde_json::to_string(&(&app_doc.styles, &app_doc.settings.theme_colors)).unwrap_or_default();
            self.styles_hash = (rev, picture_key(&s));
        }
        self.styles_hash.1
    }
}

/// The store key of a picture named by `value`: what makes two pictures one.
pub(crate) fn picture_key(value: &impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// The revision of a style-sheet preview: it changes with the style sheet, the font generation (a face
/// drawn in the fallback before its font was added), the interface language (sample letters), Switch
/// Modes' dark page and the dark theme (the dimming).
pub(crate) fn preview_revision(styles: u64, fonts: u64, lang: &str, dark_page: bool, dark: bool) -> Revision {
    Revision::of(&(styles, fonts, lang, dark_page, dark))
}

/// [`preview_revision`] for the document `app` shows, in the interface `mode`.
pub(crate) fn style_revision(app: &mut WordApp, mode: Mode) -> Revision {
    // The styles hash already covers everything a preview reads of the document: keying on the
    // document revision too would draw every tile again on each keystroke.
    let rev = app.session.rev();
    let styles = app.previews.styles_key(&app.session.doc, rev);
    preview_revision(
        styles,
        wordcraft_fonts::FontDb::global().generation(),
        crate::i18n::current().code(),
        app.session.view.dark_mode,
        mode == Mode::Dark,
    )
}

/// Dims an opaque white page picture in the dark theme so it is not the brightest thing on screen (#663).
pub(crate) fn dim(img: &mut ColorImage) {
    const LEVEL: u32 = 0xC8;
    let scale = |c: u8| u8::try_from(u32::from(c) * LEVEL / 255).unwrap_or(u8::MAX);
    for p in &mut img.pixels {
        let [r, g, b, a] = p.to_srgba_unmultiplied();
        *p = Color32::from_rgba_unmultiplied(scale(r), scale(g), scale(b), a);
    }
}

/// Renders an equation off the frame for [`math_picture`]: the inputs of one tile, owned so the render job
/// can run on a queue lane.
pub(crate) struct MathSource {
    nodes: Arc<wordcraft_doc::math::Arg>,
    display: bool,
    pt: f32,
    aspect: [u16; 2],
    ink: Color32,
}

impl MathSource {
    /// An equation of `nodes` (`display` style, font size `pt`) for a picture `aspect[0]` wide to `aspect[1]` high,
    /// inked `ink`. The picture is an ink mask baked in `ink`, so the theme's text colour is part of its revision.
    pub(crate) fn new(nodes: wordcraft_doc::math::Arg, display: bool, pt: f32, aspect: [u16; 2], ink: Color32) -> Self {
        MathSource { nodes: Arc::new(nodes), display, pt, aspect, ink }
    }

    /// The shape the tile or button shows the picture in.
    pub(crate) fn thumb_aspect(&self) -> ThumbAspect {
        ThumbAspect::new(self.aspect[0], self.aspect[1])
    }

    /// The layout box in points for `request`: the long side is the request's side at its pixel density, the other
    /// follows the aspect.
    fn size_for(&self, request: &ThumbRequest) -> Vec2 {
        let long = request.side as f32 / request.pixels_per_point.max(f32::EPSILON);
        let (wide, high) = (f32::from(self.aspect[0].max(1)), f32::from(self.aspect[1].max(1)));
        if wide >= high { Vec2::new(long, long * high / wide) } else { Vec2::new(long * wide / high, long) }
    }
}

impl ThumbSource<u64> for MathSource {
    fn revision(&self, key: &u64, request: &ThumbRequest) -> Revision {
        // The key names the equation; the rest is what draws it differently.
        Revision::of(&(*key, self.display, self.pt.to_bits(), self.aspect, self.ink.to_array(), request.side, request.pixels_per_point.to_bits()))
    }

    fn render(&self, _key: &u64, request: &ThumbRequest) -> ThumbRender {
        let (nodes, display, pt, size, ink, ppp) =
            (Arc::clone(&self.nodes), self.display, self.pt, self.size_for(request), self.ink, request.pixels_per_point);
        ThumbRender::Later(Box::new(move |_| {
            Ok(match math_image(&nodes, display, pt, size.x, size.y, ppp) {
                Some(mut img) => {
                    for p in &mut img.pixels {
                        *p = Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), p.a());
                    }
                    LoadedPicture::Image(img)
                }
                None => LoadedPicture::Blank,
            })
        }))
    }
}

/// The store key of the equation picture named `key`; callers of [`math_picture`] and the symbol strip use it.
pub(crate) fn math_key(key: &str) -> u64 {
    picture_key(&("math", key))
}

/// The thumbnails the equation pictures are made and kept in, one set for the whole context.
pub(crate) fn math_thumbnails(ctx: &egui::Context) -> Thumbnails<u64> {
    Thumbnails::in_context(ctx, "wordcraft.math", ThumbnailOptions::new())
}

/// An equation picture that shows the framework's placeholder until the render lands.
pub(crate) fn math_picture<'a>(label: impl Into<Label>, key: u64, thumbnails: &'a Thumbnails<u64>, source: &'a MathSource) -> Picture<'a> {
    Picture::rendered(label, key, thumbnails, source)
}

/// An equation rasterised for a tile of `w`×`h` points, as an ink mask.
fn math_image(nodes: &wordcraft_doc::math::Arg, display: bool, pt: f32, w: f32, h: f32, ppp: f32) -> Option<ColorImage> {
    use wordcraft_doc::math::{Math, to_linear};
    let props = CharProps { size: Some(pt), ..Default::default() };
    let mut p = Paragraph::with_text("", props.clone());
    p.props.align = Some(wordcraft_doc::props::Align::Center);
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
    let eq = wordcraft_doc::InlineObject::Equation { linear: to_linear(nodes), display, math: Math { nodes: nodes.clone(), ..Default::default() } };
    p.insert_object(0, eq, &props).ok()?;
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    d.last_section.page_w = w;
    d.last_section.page_h = 2000.0;
    d.last_section.margin_left = 0.0;
    d.last_section.margin_right = 0.0;
    d.last_section.margin_top = 0.0;
    d.last_section.margin_bottom = 0.0;
    let l = wordcraft_layout::layout(&d, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
    let page = l.pages.first()?;
    // Centre the equation's line in the tile.
    let mid = page
        .items
        .iter()
        .find_map(|it| match it {
            wordcraft_layout::Placed::Lines { para, y, .. } => Some(y + para.height / 2.0),
            _ => None,
        })
        .unwrap_or(h / 2.0);
    let mut opts = wordcraft_render::RenderOptions::default();
    opts.display.placeholders = true;
    let img = wordcraft_render::render_area(&d, page, wordcraft_geom::Rect::new(0.0, mid - h / 2.0, w, h), ppp, &opts);
    let px = img.to_straight();
    let mut out = ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px);
    // An ink mask, tinted with the theme's text colour by `MathSource`, so the pictures read on dark tiles (#817).
    ink_mask(&mut out);
    Some(out)
}

/// Turn a snippet rendered black on white paper into an ink mask: white whose alpha is the ink's
/// coverage, on a transparent background.
pub fn ink_mask(img: &mut ColorImage) {
    for p in &mut img.pixels {
        let [r, g, b, a] = p.to_srgba_unmultiplied();
        let lum = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
        let ink = 255u32.saturating_sub(lum.min(255)) * u32::from(a) / 255;
        *p = Color32::from_white_alpha(u8::try_from(ink).unwrap_or(u8::MAX));
    }
}

/// The Font dialog's sample: `text` with `props`, then `mark` (a "2" raised or
/// lowered) when `vert` is set, drawn as the document draws text on `paper`.
pub fn font_sample(
    text: &str,
    props: &CharProps,
    mark: Option<wordcraft_doc::props::VertAlign>,
    (w, h, ppp): (f32, f32, f32),
    paper: wordcraft_doc::Rgb,
    dark: bool,
) -> Option<ColorImage> {
    let mut p = Paragraph::with_text(text, props.clone());
    if let Some(v) = mark {
        let tail = CharProps { vert_align: Some(v), ..props.clone() };
        p.text.push('2');
        p.runs.push(wordcraft_doc::Run { len: 1, props: tail });
    }
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
    snippet_blocks(
        &Document::new(),
        vec![Block::Para(p)],
        SnippetOptions { width: w, height: h, pixels_per_point: ppp, margin: 2.0, paper: Some(paper), dark },
    )
}

/// Lay out `para` (with `base`'s styles) in a `w`×`h` pt box and rasterise it.
pub fn snippet(base: &Document, para: Paragraph, w: f32, h: f32, ppp: f32, margin: f32) -> Option<ColorImage> {
    snippet_blocks(base, vec![Block::Para(para)], SnippetOptions { width: w, height: h, pixels_per_point: ppp, margin, paper: None, dark: false })
}

/// [`snippet`] on the dark page of View › Switch Modes when `dark` (the Styles gallery, #663).
pub fn style_snippet(base: &Document, para: Paragraph, w: f32, h: f32, ppp: f32, margin: f32, dark: bool) -> Option<ColorImage> {
    snippet_blocks(base, vec![Block::Para(para)], SnippetOptions { width: w, height: h, pixels_per_point: ppp, margin, paper: None, dark })
}

/// How a snippet is laid out and drawn.
pub(crate) struct SnippetOptions {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) pixels_per_point: f32,
    pub(crate) margin: f32,
    pub(crate) paper: Option<wordcraft_doc::Rgb>,
    pub(crate) dark: bool,
}

/// `blocks` (with `base`'s styles and numbering) laid out in a page of the options' size and rasterised.
pub(crate) fn snippet_blocks(base: &Document, blocks: Vec<Block>, options: SnippetOptions) -> Option<ColorImage> {
    let SnippetOptions { width: w, height: h, pixels_per_point: ppp, margin, paper, dark } = options;
    let mut d = Document::new();
    d.styles = base.styles.clone();
    d.numbering = base.numbering.clone();
    d.body = blocks.into_iter().map(Arc::new).collect();
    d.last_section.page_w = w + 2.0 * margin;
    d.last_section.page_h = 2000.0;
    d.last_section.margin_left = margin;
    d.last_section.margin_right = margin;
    d.last_section.margin_top = margin;
    d.last_section.margin_bottom = 0.0;
    let l = wordcraft_layout::layout(&d, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
    let page = l.pages.first()?;
    let mut opts = crate::canvas::screen_render_options();
    if let Some(p) = paper {
        opts.paper = p;
    }
    opts.dark = dark;
    let img = wordcraft_render::render_area(&d, page, wordcraft_geom::Rect::new(0.0, 0.0, w + 2.0 * margin, h + margin), ppp, &opts);
    let px = img.to_straight();
    Some(ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px))
}

/// One character formatted by `chr` (at its size, or fitted to `w`×`h` points without one), centred in the
/// picture: laid out like document text, so symbol fonts show what the page will (Define New Bullet, #2786).
pub fn glyph_image(ch: &str, chr: &CharProps, (w, h, ppp): (f32, f32, f32), dark: bool) -> Option<ColorImage> {
    let chr = CharProps { size: Some(chr.size.unwrap_or(h * 0.7)), ..chr.clone() };
    let mut p = Paragraph::with_text(ch, chr);
    p.props.align = Some(wordcraft_doc::props::Align::Center);
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    snippet_blocks(
        &Document::new(),
        vec![Block::Para(p)],
        SnippetOptions { width: w, height: h, pixels_per_point: ppp, margin: 0.0, paper: None, dark },
    )
}

/// Manage Styles' preview of one style (`ty`: the `styles.manage` type) in `size` points: a line of sample
/// text, or a small table for a table style. A dark interface `mode` dims a light page.
pub fn style_preview_image(app: &mut WordApp, id: &str, ty: &str, size: Vec2, ppp: f32, mode: Mode) -> Option<ColorImage> {
    // The snippet adds its margin (4 pt) on both sides and below, so the picture is `size` in all.
    let (w, h) = ((size.x - 8.0).max(1.0), (size.y - 4.0).max(1.0));
    let block = match ty {
        "table" => {
            let mut tb = Table::new(3, 4, (w - 4.0).max(1.0));
            tb.props.style = Some(id.to_string());
            for (ri, row) in tb.rows.iter_mut().enumerate() {
                for (ci, cell) in row.cells.iter_mut().enumerate() {
                    let txt = match (ri, ci) {
                        (0, c) => ["", "A", "B", "C"].get(c).copied().unwrap_or(""),
                        (_, 0) => "Row",
                        _ => "1",
                    };
                    cell.blocks = vec![para_block(Paragraph::with_text(txt, CharProps::default()))];
                }
            }
            Block::Table(tb)
        }
        "character" => Block::Para(Paragraph::with_text("AaBbCcYyZz", CharProps { style: Some(id.to_string()), ..CharProps::default() })),
        _ => {
            let mut p = Paragraph::with_text("AaBbCcYyZz", CharProps::default()).styled(id);
            p.props.space_before = Some(0.0);
            p.props.indent_left = Some(0.0);
            p.props.indent_first = Some(0.0);
            p.props.numbering = None;
            Block::Para(p)
        }
    };
    let dark_page = app.session.view.dark_mode;
    let mut img = snippet_blocks(
        &app.session.doc,
        vec![block],
        SnippetOptions { width: w, height: h, pixels_per_point: ppp, margin: 4.0, paper: None, dark: dark_page },
    )?;
    if mode == Mode::Dark && !dark_page {
        dim(&mut img);
    }
    Some(img)
}

#[cfg(test)]
mod tests;

//! Rendered previews: the Styles gallery, font menu entries, table and style-set tiles. Each is a
//! tiny document laid out and rasterised by the real engine, cached as a texture.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Rect, Response, Sense, Stroke, TextureHandle, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Paragraph, Table, para_block};

use crate::WordApp;
use crate::theme::{Tokens, regular};

#[derive(Default)]
pub struct Previews {
    tex: HashMap<String, TextureHandle>,
    /// The font list, for the interface language it was made in.
    families: Option<(&'static str, Vec<String>)>,
    font_gen: u64,
    /// Lowercase English family → its name in that language (simsun → 宋体), and back.
    local_names: HashMap<String, String>,
    english_names: HashMap<String, String>,
    /// The Chinese, Japanese and Korean fonts of the font list (the Asian text font box).
    east_asian: Option<Vec<String>>,
    styles_hash: (u64, u64),
    /// Previews rendered this frame (rendering is spread over frames).
    budget: std::cell::Cell<u32>,
    frame: u64,
    /// The Equation tab's pictures, packed into one texture (#1002).
    math: MathAtlas,
}

/// Side of each square texture page the equation pictures share, in pixels.
const MATH_ATLAS: usize = 2048;
/// At most this many pages: enough for a frame's pictures at a high display scale (the 22
/// built-in equations alone take two pages at 3×), still a handful of textures in all.
const MATH_ATLAS_PAGES: usize = 4;
/// Transparent pixels between pictures, so linear filtering never bleeds one into the next.
const MATH_ATLAS_GAP: usize = 2;

/// Every equation picture (gallery tiles, structure buttons, symbols) packed into a few shared
/// textures ("pages"), row by row, and drawn as a part of one (#1002).
///
/// They used to be a texture each: hundreds of small textures, created as galleries opened and
/// all freed at once when the preview cache filled. On Windows (DirectX 12) the tiles then
/// showed other textures' memory: inverted pictures, red and teal fills, and even damaged
/// interface letters. A texture written in place is what egui does for its own glyphs; the
/// atlas creates at most [`MATH_ATLAS_PAGES`] textures and frees none while galleries are
/// browsed. When every page is full, the page least recently drawn from starts over, but never
/// one drawn from in the current frame (its pictures are already on screen): a frame whose
/// pictures don't fit shows the rest as linear text.
#[derive(Default)]
struct MathAtlas {
    pages: Vec<AtlasPage>,
    /// Picture key → its page and part of that page's texture (UV coordinates).
    uv: HashMap<String, (usize, Rect)>,
    /// How many times a page started over (for tests).
    wipes: u64,
}

#[derive(Default)]
struct AtlasPage {
    tex: Option<TextureHandle>,
    /// Where the next picture goes: the current row's left edge and top, and the row's height.
    x: usize,
    y: usize,
    row_h: usize,
    /// The last frame a picture of this page was handed out in.
    used: u64,
}

impl AtlasPage {
    /// A free spot for a `w`×`h` picture, or `None` when the page is full or it can't fit.
    fn place(&mut self, w: usize, h: usize) -> Option<[usize; 2]> {
        if w == 0 || h == 0 || w > MATH_ATLAS || h > MATH_ATLAS {
            return None;
        }
        let (mut x, mut y, mut row_h) = (self.x, self.y, self.row_h);
        if x.saturating_add(w) > MATH_ATLAS {
            x = 0;
            y = y.saturating_add(row_h).saturating_add(MATH_ATLAS_GAP);
            row_h = 0;
        }
        if y.saturating_add(h) > MATH_ATLAS {
            return None;
        }
        self.x = x.saturating_add(w).saturating_add(MATH_ATLAS_GAP);
        self.y = y;
        self.row_h = row_h.max(h);
        Some([x, y])
    }

    /// Forget the page's layout and clear its texture.
    fn reset(&mut self) {
        self.x = 0;
        self.y = 0;
        self.row_h = 0;
        if let Some(tex) = &mut self.tex {
            tex.set(egui::ColorImage::filled([MATH_ATLAS, MATH_ATLAS], egui::Color32::TRANSPARENT), egui::TextureOptions::LINEAR);
        }
    }
}

impl MathAtlas {
    /// The picture stored under `key`, marking its page as drawn from in `frame`.
    fn get(&mut self, frame: u64, key: &str) -> Option<(egui::TextureId, Rect)> {
        let (page, uv) = *self.uv.get(key)?;
        let p = self.pages.get_mut(page)?;
        let id = p.tex.as_ref()?.id();
        p.used = frame;
        Some((id, uv))
    }

    /// Add `img` under `key`; its texture and UV rectangle, or `None` if it can't be placed in
    /// this frame without wiping pictures already handed out in it.
    fn add(&mut self, ctx: &egui::Context, frame: u64, key: &str, img: egui::ColorImage) -> Option<(egui::TextureId, Rect)> {
        let [w, h] = img.size;
        if w == 0 || h == 0 || w > MATH_ATLAS || h > MATH_ATLAS {
            return None;
        }
        let mut spot = self.pages.iter_mut().enumerate().find_map(|(i, p)| p.place(w, h).map(|at| (i, at)));
        if spot.is_none() && self.pages.len() < MATH_ATLAS_PAGES {
            let mut p = AtlasPage::default();
            spot = p.place(w, h).map(|at| (self.pages.len(), at));
            self.pages.push(p);
        }
        if spot.is_none() {
            // Every page is full: start over the one drawn from longest ago, unless all of them
            // were drawn from in this frame.
            let (i, p) = self.pages.iter_mut().enumerate().filter(|(_, p)| p.used != frame).min_by_key(|(_, p)| p.used)?;
            p.reset();
            self.uv.retain(|_, (page, _)| *page != i);
            self.wipes = self.wipes.saturating_add(1);
            spot = p.place(w, h).map(|at| (i, at));
        }
        let (i, at) = spot?;
        let p = self.pages.get_mut(i)?;
        let tex = p.tex.get_or_insert_with(|| {
            ctx.load_texture(
                format!("math_atlas_{i}"),
                egui::ColorImage::filled([MATH_ATLAS, MATH_ATLAS], egui::Color32::TRANSPARENT),
                egui::TextureOptions::LINEAR,
            )
        });
        tex.set_partial(at, img, egui::TextureOptions::LINEAR);
        p.used = frame;
        let s = MATH_ATLAS as f32;
        let uv = Rect::from_min_max(pos2(at[0] as f32 / s, at[1] as f32 / s), pos2((at[0] + w) as f32 / s, (at[1] + h) as f32 / s));
        self.uv.insert(key.to_string(), (i, uv));
        Some((tex.id(), uv))
    }
}

impl Previews {
    /// The font list: installed families, each by its name in the interface language when it has
    /// one (宋体 for SimSun in Chinese, #505), like Word's.
    pub fn families(&mut self) -> Vec<String> {
        let lang = crate::i18n::current().code();
        let generation = wordcraft_fonts::FontDb::global().generation();
        if self.families.as_ref().is_none_or(|(l, _)| *l != lang) || self.font_gen != generation {
            self.font_gen = generation;
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

    /// Fonts were added or removed: list the families again and redraw the entries of
    /// `families` (an entry drawn before its font was added shows the fallback face).
    pub fn fonts_changed(&mut self, ctx: &egui::Context, families: impl Iterator<Item = String>) {
        self.families = None;
        self.east_asian = None;
        ctx.data_mut(|d| {
            for f in families {
                d.remove::<TextureHandle>(egui::Id::new(format!("font:{f}")));
            }
        });
    }

    /// A closure drawing a font-menu entry in its own face.
    pub fn font_preview_fn(&mut self) -> Box<dyn Fn(&mut Ui, &str) -> Response> {
        Box::new(|ui: &mut Ui, name: &str| {
            let key = format!("font:{name}");
            let tex: Option<TextureHandle> = ui.ctx().data(|d| d.get_temp::<TextureHandle>(egui::Id::new(&key)));
            let (r, resp) = ui.allocate_exact_size(vec2(260.0, crate::widgets::COMBO_PREVIEW_ROW_H), Sense::click());
            let t = Tokens::get(ui.ctx());
            if resp.hovered() {
                ui.painter().rect_filled(r, 3.0, t.hover);
            }
            let tex = tex.or_else(|| {
                // Context accessors share a lock: read input before taking the data write lock.
                let now = ui.input(|i| i.time);
                let n = ui.ctx().data_mut(|d| {
                    let c = d.get_temp_mut_or_default::<(f64, u32)>(egui::Id::new("font_preview_budget"));
                    if c.0 != now {
                        *c = (now, 0);
                    }
                    c.1 += 1;
                    c.1
                });
                if n > 12 {
                    ui.ctx().request_repaint();
                    return None;
                }
                let ppp = ui.ctx().pixels_per_point();
                let mut img = font_entry_image(name, ppp);
                ink_mask(&mut img);
                let h = ui.ctx().load_texture(&key, img, egui::TextureOptions::LINEAR);
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(&key), h.clone()));
                Some(h)
            });
            match tex {
                Some(h) => {
                    let sz = h.size_vec2() / ui.ctx().pixels_per_point();
                    ui.painter().image(
                        h.id(),
                        Rect::from_min_size(pos2(r.min.x + 6.0, r.center().y - sz.y / 2.0), sz),
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        t.text,
                    );
                }
                None => {
                    ui.painter().text(pos2(r.min.x + 8.0, r.center().y), egui::Align2::LEFT_CENTER, name, regular(12.0), t.text);
                }
            }
            resp
        })
    }

    pub(crate) fn get_or(&mut self, ctx: &egui::Context, key: &str, make: impl FnOnce() -> Option<egui::ColorImage>) -> Option<TextureHandle> {
        if let Some(t) = self.tex.get(key) {
            return Some(t.clone());
        }
        let frame = ctx.cumulative_frame_nr();
        if frame != self.frame {
            self.frame = frame;
            self.budget.set(0);
        }
        if self.budget.get() > 8 {
            ctx.request_repaint();
            return None;
        }
        self.budget.set(self.budget.get() + 1);
        let img = make()?;
        let h = ctx.load_texture(key, img, egui::TextureOptions::LINEAR);
        if self.tex.len() > 400 {
            self.tex.clear();
        }
        self.tex.insert(key.to_string(), h.clone());
        Some(h)
    }

    /// Hash of the style sheet (recomputed only when the document revision changes).
    fn styles_key(&mut self, app_doc: &Document, rev: u64) -> u64 {
        if self.styles_hash.0 != rev {
            let s = serde_json::to_string(&(&app_doc.styles, &app_doc.settings.theme_colors)).unwrap_or_default();
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            self.styles_hash = (rev, h.finish());
        }
        self.styles_hash.1
    }
}

/// An equation rasterised for a gallery tile or a button (`pt`: font size; `size`: the tile in
/// points), with its empty slots shown: the shared texture and the picture's UV rectangle in it
/// ([`MathAtlas`]). An ink mask, drawn tinted with the theme's text colour. `None` while waiting
/// for this frame's rendering budget.
pub fn math_texture(
    app: &mut WordApp,
    ctx: &egui::Context,
    key: &str,
    nodes: &wordcraft_doc::math::Arg,
    display: bool,
    pt: f32,
    size: egui::Vec2,
) -> Option<(egui::TextureId, Rect)> {
    let ppp = ctx.pixels_per_point();
    let k = format!("math:{key}:{pt}:{}x{}:{ppp}", size.x, size.y);
    let p = &mut app.previews;
    let frame = ctx.cumulative_frame_nr();
    if let Some(hit) = p.math.get(frame, &k) {
        return Some(hit);
    }
    if frame != p.frame {
        p.frame = frame;
        p.budget.set(0);
    }
    if p.budget.get() > 8 {
        ctx.request_repaint();
        return None;
    }
    p.budget.set(p.budget.get() + 1);
    let img = math_image(nodes, display, pt, size.x, size.y, ppp)?;
    p.math.add(ctx, frame, &k, img)
}

fn math_image(nodes: &wordcraft_doc::math::Arg, display: bool, pt: f32, w: f32, h: f32, ppp: f32) -> Option<egui::ColorImage> {
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
    let img = wordcraft_render::render_area(&d, page, 0.0, mid - h / 2.0, w, h, ppp, &opts);
    let px = img.to_straight();
    let mut out = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px);
    // An ink mask, tinted with the theme's text colour when drawn, so the pictures read on dark tiles (#817).
    ink_mask(&mut out);
    Some(out)
}

/// Turn a snippet rendered black on white paper into an ink mask: white whose alpha is the ink's
/// coverage, on a transparent background. Painted with the theme's text colour as the tint, a font
/// menu entry then reads like the rest of the menu in light and dark mode instead of sitting on a
/// white strip (#535).
pub fn ink_mask(img: &mut egui::ColorImage) {
    for p in &mut img.pixels {
        let [r, g, b, a] = p.to_srgba_unmultiplied();
        let lum = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
        let ink = 255u32.saturating_sub(lum.min(255)) * u32::from(a) / 255;
        *p = egui::Color32::from_white_alpha(u8::try_from(ink).unwrap_or(u8::MAX));
    }
}

/// A font list entry: `name` in its own face, 240×20 pt. The face is read for the preview alone
/// and dropped (#848): browsing the list used to load every family shown for good, so each trip
/// through the Font box grew memory by tens of megabytes. A name the font has no glyphs for (a
/// symbol font's), or a font that can't be read, shows in the default face.
pub fn font_entry_image(name: &str, ppp: f32) -> egui::ColorImage {
    let db = wordcraft_fonts::FontDb::global();
    let face = db
        .preview_face(name)
        .filter(|f| name.chars().filter(|c| !c.is_whitespace()).all(|c| f.covers(c)))
        .unwrap_or_else(|| db.face(wordcraft_fonts::FALLBACK_FAMILY, "Regular"));
    let img = wordcraft_render::render_text_line(&face, name, 13.0, 244.0, 22.0, 2.0, ppp);
    egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_straight())
}

/// Lay out `para` (with `base`'s styles) in a `w`×`h` pt box and rasterise it.
pub fn snippet(base: &Document, para: Paragraph, w: f32, h: f32, ppp: f32, margin: f32) -> Option<egui::ColorImage> {
    snippet_blocks(base, vec![Block::Para(para)], w, h, ppp, margin, None, false)
}

/// [`snippet`] on the dark page of View › Switch Modes when `dark` (the Styles gallery, #663).
pub fn style_snippet(base: &Document, para: Paragraph, w: f32, h: f32, ppp: f32, margin: f32, dark: bool) -> Option<egui::ColorImage> {
    snippet_blocks(base, vec![Block::Para(para)], w, h, ppp, margin, None, dark)
}

/// How a gallery draws a page preview: as rendered on a light interface or on Switch Modes' dark
/// page, and dimmed in the dark theme so a white page isn't the brightest thing on screen (#663).
pub fn preview_tint(t: &Tokens, dark_page: bool) -> egui::Color32 {
    if t.dark && !dark_page { egui::Color32::from_gray(0xC8) } else { egui::Color32::WHITE }
}

fn snippet_blocks(
    base: &Document,
    blocks: Vec<Block>,
    w: f32,
    h: f32,
    ppp: f32,
    margin: f32,
    paper: Option<wordcraft_doc::Rgb>,
    dark: bool,
) -> Option<egui::ColorImage> {
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
    let img = wordcraft_render::render_area(&d, page, 0.0, 0.0, w + 2.0 * margin, h + margin, ppp, &opts);
    let px = img.to_straight();
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px))
}

/// Manage Styles' preview of one style (`ty`: the `styles.manage` type), `size` points wide and
/// tall: a line of sample text, or a small table for a table style.
pub fn style_preview(app: &mut WordApp, ui: &mut Ui, id: &str, ty: &str, size: egui::Vec2) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(r, 3.0, preview_tint(&t, false), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let rev = app.session.rev();
    let skey = app.previews.styles_key(&app.session.doc, rev);
    let ppp = ui.ctx().pixels_per_point();
    let (w, h) = (size.x - 12.0, size.y - 8.0);
    let key = format!("manage:{id}:{ty}:{skey}:{ppp}:{w}x{h}");
    let doc = &app.session.doc;
    let tex = app.previews.get_or(ui.ctx(), &key, || {
        let block = match ty {
            "table" => {
                let mut tb = Table::new(3, 4, w - 4.0);
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
        snippet_blocks(doc, vec![block], w, h, ppp, 4.0, None, false)
    });
    if let Some(hd) = tex {
        let sz = hd.size_vec2() / ppp;
        let ir = Rect::from_min_size(pos2(r.min.x + 2.0, r.min.y + 2.0), sz.min(r.size() - vec2(4.0, 4.0)));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((ir.width() / sz.x).min(1.0), (ir.height() / sz.y).min(1.0)));
        ui.painter().with_clip_rect(r).image(hd.id(), ir, uv, preview_tint(&t, false));
    }
}

/// Tiles the Home tab's Styles gallery shows, as designed and in a compact group.
const STYLE_TILES: usize = 6;
const STYLE_TILES_COMPACT: usize = 3;

/// The Home tab's Styles gallery.
pub fn style_gallery(app: &mut WordApp, ui: &mut Ui, state: &Value) {
    let t = Tokens::get(ui.ctx());
    let current = state.get("style").and_then(Value::as_str).unwrap_or("Normal").to_string();
    let styles: Vec<(String, String)> = app.session.doc.styles.gallery().iter().map(|s| (s.id.clone(), crate::i18n::style_name(s))).collect();
    let rev = app.session.rev();
    let skey = app.previews.styles_key(&app.session.doc, rev);
    let ppp = ui.ctx().pixels_per_point();
    let start = ui.data(|d| d.get_temp::<usize>(egui::Id::new("gallery_start"))).unwrap_or(0).min(styles.len().saturating_sub(1));
    // A compact ribbon group (#662, #1285) trims the gallery to fewer tiles before it folds.
    let visible = if crate::ribbon_fit::compact(ui.ctx()) { STYLE_TILES_COMPACT } else { STYLE_TILES };
    crate::ribbon_fit::add_saving(ui.ctx(), (STYLE_TILES - STYLE_TILES_COMPACT) as f32 * 76.0);
    let (area, _) = ui.allocate_exact_size(vec2(visible as f32 * 76.0 + 18.0, crate::widgets::CONTENT_H), Sense::hover());
    ui.painter().rect(area.shrink(1.0), 4.0, t.input, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    for (k, (id, name)) in styles.iter().skip(start).take(visible).enumerate() {
        let r = Rect::from_min_size(pos2(area.min.x + 3.0 + k as f32 * 76.0, area.min.y + 3.0), vec2(74.0, area.height() - 6.0));
        let resp = ui.interact(r, ui.id().with(("style_tile", id)), Sense::click());
        let active = *id == current;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, name));
        if active {
            ui.painter().rect(r, 3.0, t.checked, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        let lang = crate::i18n::current().code();
        let dark_page = app.session.view.dark_mode;
        let key = format!("style:{id}:{skey}:{ppp}:{lang}:{dark_page}");
        let doc = &app.session.doc;
        let tex = app.previews.get_or(ui.ctx(), &key, || {
            // Sample letters in the interface's script (`АаБбВвГг` in Russian).
            let mut p = Paragraph::with_text(tl!("AaBbCcDd"), CharProps::default()).styled(id);
            p.props.space_before = Some(0.0);
            p.props.align = Some(wordcraft_doc::Align::Left);
            // Keep hanging indents (bullets) but start the first line at the tile's left edge.
            let first = doc.styles.resolve_para(&p.props).indent_first;
            p.props.indent_left = Some(if first.is_finite() { (-first).max(0.0) } else { 0.0 });
            p.props.borders = None;
            p.props.shading = None;
            style_snippet(doc, p, 220.0, 30.0, ppp, 1.0, dark_page)
        });
        if let Some(h) = tex {
            let sz = h.size_vec2() / ppp;
            let ir = Rect::from_min_size(pos2(r.min.x + 3.0, r.min.y + 4.0), sz.min(vec2(68.0, 32.0)));
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((ir.width() / sz.x).min(1.0), (ir.height() / sz.y).min(1.0)));
            ui.painter().with_clip_rect(r).image(h.id(), ir, uv, preview_tint(&t, dark_page));
        }
        let mut job = egui::text::LayoutJob::single_section(name.clone(), egui::TextFormat::simple(regular(10.5), t.text));
        job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 6.0);
        let galley = ui.painter().layout_job(job);
        let label_pos = pos2(r.center().x - galley.size().x / 2.0, r.max.y - 9.0 - galley.size().y / 2.0);
        ui.painter().with_clip_rect(r).galley(label_pos, galley, t.text);
        // Right-click a tile: the same style actions as the Styles pane.
        resp.context_menu(|ui| {
            if ui.button(tl!("Update to Match Selection")).clicked() {
                let _ = app.run("styles.updateToMatch", json!({"style": id}));
                ui.close();
            }
            if ui.button(tl!("Modify…")).clicked() {
                app.dialog = crate::dialogs::Dialog::modify_style(app, id);
                ui.close();
            }
        });
        if resp.on_hover_text(name).clicked() {
            let _ = app.run("para.style", json!({"style": id}));
        }
    }
    // Scroll arrows and "more".
    let ar = Rect::from_min_max(pos2(area.max.x - 16.0, area.min.y + 2.0), pos2(area.max.x - 2.0, area.max.y - 2.0));
    let thirds = ar.height() / 3.0;
    for (i, (icon, delta)) in [("chevronUp", -1i32), ("chevronDown", 1), ("more", 0)].iter().enumerate() {
        let br = Rect::from_min_size(pos2(ar.min.x, ar.min.y + thirds * i as f32), vec2(ar.width(), thirds));
        let resp = ui.interact(br, ui.id().with(("gal_arrow", i)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(br, 2.0, t.hover);
        }
        crate::icons::paint(ui.painter(), br.shrink(2.0), icon, t.icon, t.accent);
        if resp.clicked() {
            if *delta == 0 {
                let _ = app.run("view.stylesPane", json!({"value": true}));
            } else {
                let n = if *delta < 0 { start.saturating_sub(visible) } else { (start + visible).min(styles.len().saturating_sub(1)) };
                ui.data_mut(|d| d.insert_temp(egui::Id::new("gallery_start"), n));
            }
        }
    }
}

/// Style sets shown in the Design tab's gallery, in order.
const STYLE_SETS: [&str; 7] = ["default", "basic", "lines", "shaded", "casual", "centered", "minimalist"];
/// Width of one gallery thumbnail, points.
const STYLE_SET_W: f32 = 56.0;
/// Width the whole gallery takes with the ribbon's 1 pt item spacing.
pub const STYLE_SET_GALLERY_W: f32 = STYLE_SETS.len() as f32 * (STYLE_SET_W + 1.0) - 1.0;

/// Design tab: style set tiles.
pub fn style_set_gallery(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point();
    for name in STYLE_SETS {
        let (r, resp) = ui.allocate_exact_size(vec2(STYLE_SET_W, crate::widgets::CONTENT_H), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        let key = format!("set:{name}:{ppp}:{}", crate::i18n::current().code());
        let tex = app.previews.get_or(ui.ctx(), &key, || {
            let mut s = wordcraft_engine::Session::new(Document::new());
            let _ = s.run("design.styleSet", &json!({"name": name}));
            let blocks = vec![
                Block::Para(Paragraph::with_text(crate::i18n::tc("preview", "Title"), CharProps::default()).styled("Title")),
                Block::Para(Paragraph::with_text(tl!("Heading 1"), CharProps::default()).styled("Heading1")),
                Block::Para(Paragraph::with_text(tl!("Body text in a short paragraph to show spacing."), CharProps::default())),
            ];
            snippet_blocks(&s.doc, blocks, 120.0, 110.0, ppp * 0.42, 6.0, None, false)
        });
        if let Some(h) = tex {
            let sz = h.size_vec2() / ppp;
            let ir = Rect::from_min_size(pos2(r.min.x + 3.0, r.min.y + 3.0), sz.min(vec2(50.0, 60.0)));
            ui.painter().rect_stroke(ir, 0.0, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((ir.width() / sz.x).min(1.0), (ir.height() / sz.y).min(1.0)));
            ui.painter().image(h.id(), ir, uv, preview_tint(&t, false));
        }
        if resp.on_hover_text(format!("Style set: {name}")).clicked() {
            let _ = app.run("design.styleSet", json!({"name": name}));
        }
    }
}

/// Table Design: a tile previewing a table style.
pub fn table_style_tile(ui: &mut Ui, app: &mut WordApp, style: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point();
    let (r, resp) = ui.allocate_exact_size(vec2(58.0, 46.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    // Keyed by the style sheet too, so a modified table style redraws.
    let skey = app.previews.styles_key(&app.session.doc, app.session.rev());
    let key = format!("tstyle:{style}:{skey}:{ppp}");
    let doc = &app.session.doc;
    let tex = app.previews.get_or(ui.ctx(), &key, || {
        let mut tb = Table::new(5, 4, 100.0);
        tb.props.style = Some(style.to_string());
        for row in &mut tb.rows {
            for c in &mut row.cells {
                c.blocks = vec![para_block(Paragraph::with_text(" ", CharProps { size: Some(5.0), ..Default::default() }))];
            }
        }
        snippet_blocks(doc, vec![Block::Table(tb), Block::Para(Paragraph::new())], 104.0, 70.0, ppp * 0.48, 4.0, None, false)
    });
    if let Some(h) = tex {
        let sz = h.size_vec2() / ppp;
        let ir = Rect::from_center_size(r.center(), sz.min(vec2(54.0, 42.0)));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((ir.width() / sz.x).min(1.0), (ir.height() / sz.y).min(1.0)));
        ui.painter().image(h.id(), ir, uv, preview_tint(&t, false));
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #848: each trip through the Font box loaded every family it showed, for good. Entries are
    /// now drawn with a face read for the preview alone: installed families stay unloaded.
    #[test]
    fn font_menu_entries_leave_installed_families_unloaded() {
        let db = wordcraft_fonts::FontDb::global();
        let ink = |img: &egui::ColorImage| img.pixels.iter().filter(|p| p.r() < 128).count();
        assert!(ink(&font_entry_image("Source Sans 3", 1.0)) > 20, "the bundled face draws");
        assert!(ink(&font_entry_image("No Such Font 848", 1.0)) > 20, "an unknown font shows in the default face");
        // Tests running alongside share this database and may load a fallback (a math font for
        // an equation, a font for a script) at any time: leave the math fonts out, and allow a
        // stray load or two. Before #848 every family previewed was loaded.
        let shared = |f: &str| wordcraft_fonts::math::MATH_SUBSTITUTES.iter().any(|n| f.eq_ignore_ascii_case(n));
        let unloaded: Vec<String> = db.families().into_iter().filter(|f| !db.is_loaded(f) && !shared(f)).take(25).collect();
        let mut loaded = Vec::new();
        for f in &unloaded {
            let img = font_entry_image(f, 1.0);
            assert_eq!(img.size, [244, 22]);
            if db.is_loaded(f) {
                loaded.push(f.clone());
            }
        }
        assert!(loaded.len() <= 2, "previewing loaded {loaded:?}");
    }

    /// #663: gallery previews are dimmed in the dark theme, unless they show Switch Modes' dark
    /// page; the light theme keeps a white page.
    #[test]
    fn previews_are_dimmed_in_the_dark_theme() {
        let (light, dark) = (crate::theme::Tokens::light(), crate::theme::Tokens::dark());
        assert_eq!(super::preview_tint(&light, false), egui::Color32::WHITE);
        assert_eq!(super::preview_tint(&dark, true), egui::Color32::WHITE, "the dark page is already dark");
        let dim = super::preview_tint(&dark, false);
        assert!(dim.r() < 0xE0 && dim.r() > 0xA0, "{dim:?}");
    }

    #[test]
    fn font_menu_previews_render_defer_and_reuse_without_deadlocking() {
        // A nested Context lock used to hang the first uncached menu entry. Bound the wait so
        // a regression fails the test instead of hanging the entire test suite.
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let ctx = egui::Context::default();
            let preview = Previews::default().font_preview_fn();
            let budget_id = egui::Id::new("font_preview_budget");
            let sans_id = egui::Id::new("font:Source Sans 3");
            let inter_id = egui::Id::new("font:Inter");
            ctx.data_mut(|d| d.insert_temp(budget_id, (1.0_f64, 11_u32)));

            for time in [1.0, 2.0, 3.0] {
                let input = egui::RawInput { time: Some(time), ..Default::default() };
                ctx.run_ui(input, |ui| {
                    preview(ui, "Source Sans 3");
                    preview(ui, "Inter");
                })
                .drop_without_applying_deltas();
                ctx.data(|d| {
                    assert!(d.get_temp::<TextureHandle>(sans_id).is_some());
                    if time == 1.0 {
                        assert!(d.get_temp::<TextureHandle>(inter_id).is_none(), "over-budget entry should defer");
                        assert_eq!(d.get_temp::<(f64, u32)>(budget_id), Some((1.0, 13)));
                    } else {
                        assert!(d.get_temp::<TextureHandle>(inter_id).is_some(), "deferred entry should render next frame");
                        assert_eq!(d.get_temp::<(f64, u32)>(budget_id), Some((2.0, 1)), "cached entries should not consume budget");
                    }
                });
            }
            tx.send(()).unwrap();
        });
        rx.recv_timeout(std::time::Duration::from_secs(30)).expect("font menu preview rendering hung or failed");
        worker.join().unwrap();
    }

    /// #817: equation pictures were black on white; they are now an ink mask (white, transparent paper).
    #[test]
    fn math_pictures_are_ink_masks() {
        use wordcraft_doc::math::parse_linear;
        let img = math_image(&parse_linear("x/y"), false, 12.0, 40.0, 34.0, 1.0).expect("renders");
        assert!(img.pixels.iter().any(|p| p.a() == 0), "paper is transparent");
        assert!(img.pixels.iter().any(|p| p.a() > 0), "ink is kept");
        assert!(img.pixels.iter().all(|p| *p == egui::Color32::from_white_alpha(p.a())), "ink is white, to be tinted");
    }

    /// Every picture the Equation tab and Insert › Equation draw: (key, nodes, display, pt, tile size).
    fn equation_pictures() -> Vec<(String, wordcraft_doc::math::Arg, bool, f32, egui::Vec2)> {
        use wordcraft_doc::math::{MNode, MRun, parse_linear};
        use wordcraft_engine::math_gallery::{BUILT_INS, STRUCTURES, SYMBOL_SETS};
        let mut out = Vec::new();
        for g in STRUCTURES {
            let pic = crate::equation_tab::button_picture(g.id);
            out.push((format!("btn:{pic}"), parse_linear(pic), false, 12.0, vec2(40.0, 32.0)));
            for (_, templates) in g.sections {
                for t in templates.iter() {
                    out.push((format!("tpl:{}", t.id), t.nodes(), true, 14.0, vec2(56.0, 52.0)));
                    out.push((format!("tpl:{}", t.id), t.nodes(), true, 13.0, vec2(154.0, 56.0)));
                }
            }
        }
        for (id, _, lin) in BUILT_INS {
            out.push((format!("builtin:{id}"), parse_linear(lin), true, 11.0, vec2(346.0, 60.0)));
        }
        for (_, chars) in SYMBOL_SETS {
            for c in chars.chars() {
                let nodes = vec![MNode::Run(MRun { text: c.to_string(), lit: true, ..Default::default() })];
                out.push((format!("sym:{c}"), nodes, false, 11.0, vec2(22.0, 22.0)));
            }
        }
        out
    }

    /// #1002: drawn the way the Equation tab draws them (the picture tinted with the theme's text
    /// colour on an input-coloured tile), every gallery tile, button and symbol is the theme's
    /// greys only: its background is the tile colour and its ink no darker or lighter than the
    /// text, in light and dark mode, with no red or teal anywhere.
    #[test]
    fn equation_pictures_follow_the_theme() {
        let mix = |fg: u8, bg: u8, a: u8| ((u32::from(fg) * u32::from(a) + u32::from(bg) * (255 - u32::from(a))) / 255) as u8;
        for (key, nodes, display, pt, size) in equation_pictures() {
            let img = math_image(&nodes, display, pt, size.x, size.y, 1.0).unwrap_or_else(|| panic!("{key} renders"));
            assert!(img.pixels.iter().all(|p| *p == egui::Color32::from_white_alpha(p.a())), "{key}: an ink mask");
            // Ink may reach the corner (a matrix bracket with Windows fonts): look for paper anywhere.
            assert!(img.pixels.iter().any(|p| p.a() == 0), "{key}: transparent paper");
            for t in [Tokens::light(), Tokens::dark()] {
                let (bg, fg) = (t.input, t.text);
                let (lo, hi) = (bg.r().min(fg.r()), bg.r().max(fg.r()));
                for p in &img.pixels {
                    let a = p.a();
                    let c = [mix(fg.r(), bg.r(), a), mix(fg.g(), bg.g(), a), mix(fg.b(), bg.b(), a)];
                    assert!(c[0] == c[1] && c[1] == c[2], "{key}: a grey, not {c:?}");
                    assert!((lo..=hi).contains(&c[0]), "{key}: between the tile and the text colour, not {c:?}");
                }
            }
        }
    }

    /// #1002: the pictures share one texture, written in place: browsing every gallery creates no
    /// texture per tile and frees none (hundreds of small textures, freed in bulk, showed other
    /// textures' memory on DirectX 12).
    #[test]
    fn equation_pictures_share_one_texture() {
        let ctx = egui::Context::default();
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        let pictures = equation_pictures();
        // Symbols repeat across sets (∀, ≠…): each is drawn once.
        let unique = pictures.iter().map(|(key, _, _, pt, size)| format!("{key}:{pt}:{size:?}")).collect::<std::collections::HashSet<_>>().len();
        let mut drawn: HashMap<String, (egui::TextureId, Rect)> = HashMap::new();
        let mut freed = 0;
        for _ in 0..200 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let ctx = ui.ctx().clone();
                for (key, nodes, display, pt, size) in &pictures {
                    let k = format!("{key}:{pt}:{size:?}");
                    if let std::collections::hash_map::Entry::Vacant(e) = drawn.entry(k)
                        && let Some(t) = math_texture(&mut app, &ctx, key, nodes, *display, *pt, *size)
                    {
                        e.insert(t);
                    }
                }
            });
            freed += out.textures_delta.free.len();
            out.textures_delta.clear();
            if drawn.len() == unique {
                break;
            }
        }
        assert_eq!(drawn.len(), unique, "every picture is drawn");
        let ids: std::collections::HashSet<_> = drawn.values().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), 1, "one texture for every picture");
        assert_eq!(freed, 0, "no texture freed while browsing");
        // No two pictures overlap, and each lies inside the texture.
        let rects: Vec<Rect> = drawn.values().map(|(_, uv)| *uv).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a.min.x >= 0.0 && a.min.y >= 0.0 && a.max.x <= 1.0 && a.max.y <= 1.0, "{a:?} inside");
            for b in rects.iter().skip(i + 1) {
                assert!(!a.intersects(*b) || a.intersect(*b).area() <= 0.0, "{a:?} overlaps {b:?}");
            }
        }
    }

    /// #1002: pictures are packed row by row, and pictures that can never fit are refused.
    #[test]
    fn math_atlas_packs_rows() {
        let mut a = AtlasPage::default();
        assert_eq!(a.place(100, 40), Some([0, 0]));
        assert_eq!(a.place(100, 20), Some([100 + MATH_ATLAS_GAP, 0]));
        assert_eq!(a.place(MATH_ATLAS, 10), Some([0, 40 + MATH_ATLAS_GAP]), "a new row under the tallest");
        assert_eq!(a.place(MATH_ATLAS + 1, 10), None);
        assert_eq!(a.place(10, 0), None);
        assert_eq!(a.place(10, MATH_ATLAS), None, "full");
        assert_eq!(a.place(10, 10), Some([0, 40 + MATH_ATLAS_GAP + 10 + MATH_ATLAS_GAP]), "still room after a refusal");
    }

    /// #1002: a page is never wiped in a frame that handed out one of its pictures (they are
    /// already on screen); once all pages are full, the least recently drawn page starts over.
    #[test]
    fn math_atlas_never_wipes_a_page_drawn_from_this_frame() {
        let ctx = egui::Context::default();
        let img = || egui::ColorImage::filled([MATH_ATLAS, MATH_ATLAS], egui::Color32::TRANSPARENT);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut a = MathAtlas::default();
            let first: Vec<_> = (0..MATH_ATLAS_PAGES).map(|i| a.add(ui.ctx(), 1, &format!("p{i}"), img())).collect();
            assert!(first.iter().all(Option::is_some), "a page each");
            let ids: std::collections::HashSet<_> = first.iter().flatten().map(|(id, _)| *id).collect();
            assert_eq!(ids.len(), MATH_ATLAS_PAGES);
            assert!(a.add(ui.ctx(), 1, "more", img()).is_none(), "every page was drawn from this frame");
            assert_eq!(a.wipes, 0);
            assert!(a.uv.len() == MATH_ATLAS_PAGES, "nothing forgotten");
            // Next frame: page 0 is drawn from again, so the one drawn longest ago is page 1.
            assert_eq!(a.get(2, "p0"), first.first().copied().flatten());
            let (id, _) = a.add(ui.ctx(), 2, "more", img()).expect("a page starts over");
            assert_eq!(a.wipes, 1);
            assert_eq!(Some(id), first.get(1).copied().flatten().map(|(id, _)| id), "the least recently drawn page");
            assert!(a.uv.contains_key("p0") && !a.uv.contains_key("p1"));
            // Pages 0 and 1 are now in use this frame; 2 and 3 are free to start over, then none.
            assert!(a.add(ui.ctx(), 2, "x", img()).is_some());
            assert!(a.add(ui.ctx(), 2, "y", img()).is_some());
            assert!(a.add(ui.ctx(), 2, "z", img()).is_none());
            assert_eq!(a.wipes, 3);
            assert!(a.get(2, "p0").is_some() && a.get(2, "more").is_some() && a.get(2, "x").is_some() && a.get(2, "y").is_some());
        });
        out.textures_delta.clear();
    }

    /// #1002: at a display scale of 3, the 22 built-in equations need more than one page; the
    /// built-in menu opened again draws them all from the atlas, with nothing wiped or moved
    /// (a single page used to start over every frame: flicker and constant re-rendering).
    #[test]
    fn builtin_menu_at_scale_3_fits_without_wiping() {
        use wordcraft_doc::math::parse_linear;
        use wordcraft_engine::math_gallery::BUILT_INS;
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(3.0);
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        let builtins: Vec<_> = BUILT_INS.iter().map(|(id, _, lin)| (format!("builtin:{id}"), parse_linear(lin))).collect();
        let draw = |app: &mut WordApp| {
            let mut got = Vec::new();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(ui.ctx().pixels_per_point(), 3.0);
                let ctx = ui.ctx().clone();
                got = builtins.iter().map(|(key, nodes)| math_texture(app, &ctx, key, nodes, true, 11.0, vec2(346.0, 60.0))).collect();
            });
            out.textures_delta.clear();
            got
        };
        // The first opening renders a few a frame.
        let mut first = draw(&mut app);
        for _ in 0..20 {
            if first.iter().all(Option::is_some) {
                break;
            }
            first = draw(&mut app);
        }
        assert!(first.iter().all(Option::is_some), "every built-in drawn");
        let wipes = app.previews.math.wipes;
        let again = draw(&mut app);
        assert_eq!(again, first, "the same texture and place for each");
        assert_eq!(app.previews.math.wipes, wipes, "nothing started over");
        assert_eq!(wipes, 0);
    }

    #[test]
    fn ink_mask_keeps_the_ink_and_drops_the_paper() {
        let mut img = egui::ColorImage::new([3, 1], vec![egui::Color32::WHITE, egui::Color32::BLACK, egui::Color32::from_gray(128)]);
        ink_mask(&mut img);
        assert_eq!(img.pixels[0].a(), 0, "paper becomes transparent");
        assert_eq!(img.pixels[1], egui::Color32::from_white_alpha(255), "full ink stays opaque");
        assert_eq!(img.pixels[2].a(), 127, "antialiased ink keeps its coverage");
    }

    /// #535: font menu entries drew on a white strip in dark mode. They are now tinted with the
    /// theme's text colour and draw no background of their own.
    #[test]
    fn font_menu_entries_take_the_theme_text_colour() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx, crate::theme::Appearance::System);
            ctx.set_theme(theme);
            let preview = Previews::default().font_preview_fn();
            let mut out = None;
            for time in [1.0, 2.0] {
                let input = egui::RawInput { time: Some(time), ..Default::default() };
                let mut o = ctx.run_ui(input, |ui| {
                    preview(ui, "Source Sans 3");
                });
                o.textures_delta.clear();
                out = Some(o);
            }
            let want = if theme == egui::Theme::Dark { Tokens::dark().text } else { Tokens::light().text };
            let shapes = out.map(|o| o.shapes).unwrap_or_default();
            let mut images = 0;
            for s in &shapes {
                match &s.shape {
                    egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
                        images += 1;
                        assert!(m.vertices.iter().all(|v| v.color == want), "{theme:?}: the entry is not tinted with the text colour");
                    }
                    egui::Shape::Rect(r) => panic!("{theme:?}: an idle entry paints no background, got {:?}", r.fill),
                    _ => {}
                }
            }
            assert_eq!(images, 1, "{theme:?}: the entry draws its rendered name");
        }
    }
}

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
    families: Option<Vec<String>>,
    styles_hash: (u64, u64),
    /// Previews rendered this frame (rendering is spread over frames).
    budget: std::cell::Cell<u32>,
    frame: u64,
}

impl Previews {
    pub fn families(&mut self) -> Vec<String> {
        if self.families.is_none() {
            let mut f: Vec<String> = wordcraft_fonts::FontDb::global().families().into_iter().filter(|f| !f.starts_with('.')).collect();
            for w in [wordcraft_doc::styles::BODY_FONT, wordcraft_doc::styles::HEADING_FONT] {
                if !f.iter().any(|x| x == w) {
                    f.push(w.to_string());
                }
            }
            f.sort_by_key(|a| a.to_lowercase());
            f.dedup();
            self.families = Some(f);
        }
        self.families.clone().unwrap_or_default()
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
                let img = snippet(
                    &Document::new(),
                    Paragraph::with_text(name, CharProps { font: Some(name.to_string()), size: Some(13.0), ..Default::default() }),
                    240.0,
                    20.0,
                    ppp,
                    2.0,
                )?;
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
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().text(pos2(r.min.x + 8.0, r.center().y), egui::Align2::LEFT_CENTER, name, regular(12.0), t.text);
                }
            }
            resp
        })
    }

    fn get_or(&mut self, ctx: &egui::Context, key: &str, make: impl FnOnce() -> Option<egui::ColorImage>) -> Option<TextureHandle> {
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
/// points), with its empty slots shown. Cached by `key`; `None` while waiting for this frame's
/// rendering budget.
pub fn math_texture(
    app: &mut WordApp,
    ctx: &egui::Context,
    key: &str,
    nodes: &wordcraft_doc::math::Arg,
    display: bool,
    pt: f32,
    size: egui::Vec2,
) -> Option<TextureHandle> {
    let ppp = ctx.pixels_per_point();
    let k = format!("math:{key}:{pt}:{}x{}:{ppp}", size.x, size.y);
    app.previews.get_or(ctx, &k, || math_image(nodes, display, pt, size.x, size.y, ppp))
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
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px))
}

/// Lay out `para` (with `base`'s styles) in a `w`×`h` pt box and rasterise it.
pub fn snippet(base: &Document, para: Paragraph, w: f32, h: f32, ppp: f32, margin: f32) -> Option<egui::ColorImage> {
    snippet_blocks(base, vec![Block::Para(para)], w, h, ppp, margin, None)
}

fn snippet_blocks(
    base: &Document,
    blocks: Vec<Block>,
    w: f32,
    h: f32,
    ppp: f32,
    margin: f32,
    paper: Option<wordcraft_doc::Rgb>,
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
    let img = wordcraft_render::render_area(&d, page, 0.0, 0.0, w + 2.0 * margin, h + margin, ppp, &opts);
    let px = img.to_straight();
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px))
}

/// The Home tab's Styles gallery.
pub fn style_gallery(app: &mut WordApp, ui: &mut Ui, state: &Value) {
    let t = Tokens::get(ui.ctx());
    let current = state.get("style").and_then(Value::as_str).unwrap_or("Normal").to_string();
    let styles: Vec<(String, String)> = app.session.doc.styles.gallery().iter().map(|s| (s.id.clone(), s.name.clone())).collect();
    let rev = app.session.rev();
    let skey = app.previews.styles_key(&app.session.doc, rev);
    let ppp = ui.ctx().pixels_per_point();
    let start = ui.data(|d| d.get_temp::<usize>(egui::Id::new("gallery_start"))).unwrap_or(0).min(styles.len().saturating_sub(1));
    let visible = 6usize;
    let (area, _) = ui.allocate_exact_size(vec2(visible as f32 * 76.0 + 18.0, crate::widgets::CONTENT_H), Sense::hover());
    ui.painter().rect(area.shrink(1.0), 4.0, t.input, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    for (k, (id, name)) in styles.iter().skip(start).take(visible).enumerate() {
        let r = Rect::from_min_size(pos2(area.min.x + 3.0 + k as f32 * 76.0, area.min.y + 3.0), vec2(74.0, area.height() - 6.0));
        let resp = ui.interact(r, ui.id().with(("style_tile", id)), Sense::click());
        let active = *id == current;
        if active {
            ui.painter().rect(r, 3.0, t.checked, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        let key = format!("style:{id}:{skey}:{ppp}");
        let doc = &app.session.doc;
        let tex = app.previews.get_or(ui.ctx(), &key, || {
            let mut p = Paragraph::with_text("AaBbCcDd", CharProps::default()).styled(id);
            p.props.space_before = Some(0.0);
            p.props.align = Some(wordcraft_doc::Align::Left);
            // Keep hanging indents (bullets) but start the first line at the tile's left edge.
            let first = doc.styles.resolve_para(&p.props).indent_first;
            p.props.indent_left = Some(if first.is_finite() { (-first).max(0.0) } else { 0.0 });
            p.props.borders = None;
            p.props.shading = None;
            snippet(doc, p, 220.0, 30.0, ppp, 1.0)
        });
        if let Some(h) = tex {
            let sz = h.size_vec2() / ppp;
            let ir = Rect::from_min_size(pos2(r.min.x + 3.0, r.min.y + 4.0), sz.min(vec2(68.0, 32.0)));
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((ir.width() / sz.x).min(1.0), (ir.height() / sz.y).min(1.0)));
            ui.painter().with_clip_rect(r).image(h.id(), ir, uv, egui::Color32::WHITE);
        }
        let mut job = egui::text::LayoutJob::single_section(name.clone(), egui::TextFormat::simple(regular(10.5), t.text));
        job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 6.0);
        let galley = ui.painter().layout_job(job);
        let label_pos = pos2(r.center().x - galley.size().x / 2.0, r.max.y - 9.0 - galley.size().y / 2.0);
        ui.painter().with_clip_rect(r).galley(label_pos, galley, t.text);
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

/// Design tab: style set tiles.
pub fn style_set_gallery(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ppp = ui.ctx().pixels_per_point();
    for name in ["default", "basic", "lines", "shaded", "casual", "centered", "minimalist"] {
        let (r, resp) = ui.allocate_exact_size(vec2(56.0, crate::widgets::CONTENT_H), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        let key = format!("set:{name}:{ppp}");
        let tex = app.previews.get_or(ui.ctx(), &key, || {
            let mut s = wordcraft_engine::Session::new(Document::new());
            let _ = s.run("design.styleSet", &json!({"name": name}));
            let blocks = vec![
                Block::Para(Paragraph::with_text("Title", CharProps::default()).styled("Title")),
                Block::Para(Paragraph::with_text("Heading 1", CharProps::default()).styled("Heading1")),
                Block::Para(Paragraph::with_text("Body text in a short paragraph to show spacing.", CharProps::default())),
            ];
            snippet_blocks(&s.doc, blocks, 120.0, 110.0, ppp * 0.42, 6.0, None)
        });
        if let Some(h) = tex {
            let sz = h.size_vec2() / ppp;
            let ir = Rect::from_min_size(pos2(r.min.x + 3.0, r.min.y + 3.0), sz.min(vec2(50.0, 60.0)));
            ui.painter().rect_stroke(ir, 0.0, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((ir.width() / sz.x).min(1.0), (ir.height() / sz.y).min(1.0)));
            ui.painter().image(h.id(), ir, uv, egui::Color32::WHITE);
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
    let key = format!("tstyle:{style}:{ppp}");
    let doc = &app.session.doc;
    let tex = app.previews.get_or(ui.ctx(), &key, || {
        let mut tb = Table::new(5, 4, 100.0);
        tb.props.style = Some(style.to_string());
        for row in &mut tb.rows {
            for c in &mut row.cells {
                c.blocks = vec![para_block(Paragraph::with_text(" ", CharProps { size: Some(5.0), ..Default::default() }))];
            }
        }
        snippet_blocks(doc, vec![Block::Table(tb), Block::Para(Paragraph::new())], 104.0, 70.0, ppp * 0.48, 4.0, None)
    });
    if let Some(h) = tex {
        let sz = h.size_vec2() / ppp;
        let ir = Rect::from_center_size(r.center(), sz.min(vec2(54.0, 42.0)));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((ir.width() / sz.x).min(1.0), (ir.height() / sz.y).min(1.0)));
        ui.painter().image(h.id(), ir, uv, egui::Color32::WHITE);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

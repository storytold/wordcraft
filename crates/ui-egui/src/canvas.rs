//! The document canvas: pages on a grey background, rendered by the engine into textures (one
//! per page, re-rendered only when the page's content changes), with the caret, selection,
//! rulers and mouse editing drawn by egui on top.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureHandle, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::{PartKind, Pos, StoryRef};
use wordcraft_layout::{DocLayout, Page, Placed};

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};

/// Points on screen per document point at 100% (96 px per inch, like a word processor).
pub const PX_PER_PT: f32 = 96.0 / 72.0;
const GAP: f32 = 18.0;
const RULER: f32 = 22.0;

pub struct CanvasState {
    textures: HashMap<usize, (u64, TextureHandle)>,
    pub scroll_to_caret: bool,
    pub caret_visible_since: f64,
    dragging: bool,
    pub last_highlight: String,
    pub last_font_color: String,
    pub last_shading: String,
    pub open_url: Option<String>,
    /// Screen rects of pages last frame (for the control channel: page ↔ screen coordinates).
    pub page_rects: Vec<Rect>,
    /// Effective scale (screen points per document point).
    pub scale: f32,
    pub canvas_rect: Option<Rect>,
    pub focused: bool,
    pub render_ms: f64,
    pub ime_preedit: String,
    pub want_focus: bool,
    pub context_issue: Option<serde_json::Value>,
    pub context_synonyms: Option<serde_json::Value>,
    /// The right-click context menu is open (the mini toolbar stands down while it is).
    pub context_menu_open: bool,
    /// Screen rect of the selection's first line, for the floating mini toolbar.
    pub mini_anchor: Option<Rect>,
    /// A picture, shape or text box being dragged by its frame.
    pub(crate) obj_drag: Option<crate::objects::ObjectDrag>,
    /// Pages per row last frame; when it changes the caret's page is scrolled back into view.
    pub cols: usize,
}

impl CanvasState {
    /// The cached rendering of a page.
    pub(crate) fn page_texture(&self, page: usize) -> Option<&TextureHandle> {
        self.textures.get(&page).map(|(_, t)| t)
    }
}

impl Default for CanvasState {
    fn default() -> Self {
        CanvasState {
            textures: HashMap::new(),
            scroll_to_caret: true,
            caret_visible_since: 0.0,
            dragging: false,
            last_highlight: "yellow".into(),
            last_font_color: "C00000".into(),
            last_shading: "FFF2CC".into(),
            open_url: None,
            page_rects: Vec::new(),
            scale: PX_PER_PT,
            canvas_rect: None,
            focused: false,
            render_ms: 0.0,
            ime_preedit: String::new(),
            want_focus: true,
            context_issue: None,
            context_synonyms: None,
            obj_drag: None,
            context_menu_open: false,
            mini_anchor: None,
            cols: 1,
        }
    }
}

impl CanvasState {
    /// True while a mouse drag-select is in progress (the mini toolbar waits for mouse-up).
    pub fn drag_selecting(&self) -> bool {
        self.dragging
    }
}

/// Page positions in content space (points), for the current zoom.
pub struct Geometry {
    pub rects: Vec<Rect>,
    pub size: egui::Vec2,
    pub scale: f32,
    /// Pages per row.
    pub cols: usize,
}

/// Width of the markup area beside each page for comment balloons (points), or 0.
pub fn markup_width(app: &WordApp) -> f32 {
    let v = &app.session.view;
    // Word shows comments either in balloons (contextual) or in the Comments pane (list).
    let on = v.show_markup && !v.comments_pane && !v.read_mode && !v.multi_page && v.mode == wordcraft_layout::ViewMode::Print;
    if on && !app.session.doc.comments.is_empty() { 216.0 } else { 0.0 }
}

/// A page dimension (points) safe to lay out: finite, at least 1 pt, at most `cap`.
fn sane_dim(v: f32, cap: f32) -> f32 {
    if v.is_finite() { v.clamp(1.0, cap) } else { 72.0 }
}

/// How many pages fit side by side: as many slots of `slot_px` (a page plus its markup area, on
/// screen) as the width holds with a gap around each, like Word's Print Layout when zoomed out.
/// At least `min_cols`, never more than `pages` (or 1 when `auto` is off and `min_cols` is 1).
fn columns_for(avail_w: f32, slot_px: f32, pages: usize, min_cols: usize, auto: bool) -> usize {
    let fit = if auto && avail_w.is_finite() && slot_px.is_finite() && slot_px > 0.0 {
        // Saturating float → int cast; bounded by the page count below.
        ((avail_w - GAP) / (slot_px + GAP)).floor().max(1.0) as usize
    } else {
        1
    };
    fit.max(min_cols).min(pages.max(1)).max(1)
}

/// Place pages (sizes in points) in rows of `cols`, left to right then top to bottom, each in a slot
/// as wide as the widest page plus `markup` and centred in `avail_w`. Returns the screen-space rects
/// (relative to the content's top-left) and the content size.
fn place_pages(sizes: &[(f32, f32)], markup: f32, scale: f32, cols: usize, avail: egui::Vec2) -> (Vec<Rect>, egui::Vec2) {
    let cols = cols.max(1);
    let scale = if scale.is_finite() { scale.clamp(0.01, 10.0) } else { PX_PER_PT };
    let markup = if markup.is_finite() { markup.clamp(0.0, 10_000.0) } else { 0.0 };
    let avail = vec2(if avail.x.is_finite() { avail.x.max(0.0) } else { 0.0 }, if avail.y.is_finite() { avail.y.max(0.0) } else { 0.0 });
    let maxw = sizes.iter().map(|&(w, _)| sane_dim(w, 1e5)).fold(0.0f32, f32::max).max(72.0) + markup;
    let slot = maxw * scale;
    let row_w = cols as f32 * slot + (cols as f32 - 1.0) * GAP;
    let content_w = (row_w + 2.0 * GAP).max(avail.x);
    let mut rects = Vec::with_capacity(sizes.len());
    let mut y = GAP;
    for row in sizes.chunks(cols) {
        let mut x = (content_w - row_w) / 2.0;
        let mut h = 0.0f32;
        for &(w, ph) in row {
            let (w, ph) = (sane_dim(w, 1e5), sane_dim(ph, 1e6));
            // Pages narrower than the widest keep their markup area beside them.
            let px = x + (maxw - markup - w).max(0.0) * scale / 2.0;
            rects.push(Rect::from_min_size(pos2(px, y), vec2(w * scale, ph * scale)));
            h = h.max(ph * scale);
            x += slot + GAP;
        }
        y += h + GAP;
    }
    (rects, vec2(content_w, y.max(avail.y)))
}

pub fn geometry(app: &WordApp, l: &DocLayout, avail: egui::Vec2) -> Geometry {
    let v = &app.session.view;
    let markup = markup_width(app);
    let maxw = l.pages.iter().map(|p| sane_dim(p.w, 1e5)).fold(0.0f32, f32::max).max(72.0) + markup;
    let maxh = l.pages.iter().map(|p| sane_dim(p.h, 20_000.0)).fold(0.0f32, f32::max).max(72.0);
    let web = v.mode != wordcraft_layout::ViewMode::Print;
    let mut scale = v.zoom.clamp(0.1, 5.0) * PX_PER_PT;
    match v.fit.as_str() {
        "pageWidth" => scale = ((avail.x - 2.0 * GAP - 20.0) / maxw).clamp(0.1, 6.0),
        "onePage" => scale = ((avail.y - 2.0 * GAP) / maxh).min((avail.x - 2.0 * GAP) / maxw).clamp(0.05, 6.0),
        "multiplePages" => scale = ((avail.y - 2.0 * GAP) / maxh).min((avail.x - 3.0 * GAP) / (2.0 * maxw)).clamp(0.05, 6.0),
        _ => {}
    }
    if v.read_mode {
        scale = ((avail.y - 2.0 * GAP) / maxh).min((avail.x - 3.0 * GAP) / (2.0 * maxw)).clamp(0.05, 6.0);
    }
    if web {
        scale = PX_PER_PT * v.zoom.clamp(0.1, 5.0);
    }
    if !scale.is_finite() {
        scale = PX_PER_PT;
    }
    // Pages flow side by side in Print Layout whenever more than one fits across (Read Mode is a
    // two-page spread; One Page and Page Width show one page per row; Web Layout has no pages).
    let (min_cols, auto) = if v.read_mode {
        (2, false)
    } else if web || v.fit == "onePage" || v.fit == "pageWidth" {
        (1, false)
    } else if v.multi_page {
        (2, true)
    } else {
        (1, true)
    };
    let cols = if v.read_mode { 2 } else { columns_for(avail.x, maxw * scale, l.pages.len(), min_cols, auto) };
    let sizes: Vec<(f32, f32)> = l.pages.iter().map(|p| (p.w, p.h)).collect();
    let (rects, size) = place_pages(&sizes, markup, scale, cols, avail);
    Geometry { rects, size, scale, cols }
}

/// While a fit mode (Page Width, One Page, Multiple Pages) sizes the page, keep the session's zoom
/// equal to what is shown, as Word does. Zoom In/Out, the Zoom dialog and `view.state` then start
/// from the visible zoom; before, they stepped from the stale manual zoom, so Zoom In from a 163%
/// Page Width jumped to 110% (issue #67).
pub fn sync_fit_zoom(app: &mut WordApp, scale: f32) {
    let v = &mut app.session.view;
    if v.fit.is_empty() || v.read_mode || v.mode != wordcraft_layout::ViewMode::Print || !scale.is_finite() {
        return;
    }
    v.zoom = (scale / PX_PER_PT).clamp(0.1, 5.0);
}

/// Fingerprint of a page's content for the texture cache.
fn page_key(app: &WordApp, page: &Page, scale_px: f32, dim_body: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    scale_px.to_bits().hash(&mut h);
    let v = &app.session.view;
    (v.marks, v.show_markup, v.dark_mode, wordcraft_render::DARK_PAPER).hash(&mut h);
    dim_body.hash(&mut h);
    format!("{:?}{:?}", app.session.doc.settings.page_color, app.session.doc.settings.watermark).hash(&mut h);
    (page.w.to_bits(), page.h.to_bits()).hash(&mut h);
    for list in [&page.items, &page.header, &page.footer] {
        list.len().hash(&mut h);
        for it in list.iter() {
            match it {
                Placed::Lines { para, l0, l1, x, y, .. } => {
                    (std::sync::Arc::as_ptr(para) as usize, l0, l1, x.to_bits(), y.to_bits()).hash(&mut h);
                }
                Placed::Fill { rect, color } => format!("{rect:?}{color:?}").hash(&mut h),
                Placed::Rule { x0, y0, x1, y1, border } => format!("{x0}{y0}{x1}{y1}{border:?}").hash(&mut h),
                Placed::Image { rect, media, .. } => format!("{rect:?}{media}").hash(&mut h),
                Placed::Shape { rect, kind, fill, stroke, .. } => format!("{rect:?}{kind:?}{fill:?}{stroke:?}").hash(&mut h),
                Placed::Cell { .. } | Placed::Object { .. } => {}
            }
        }
    }
    app.session.doc.media.len().hash(&mut h);
    h.finish()
}

fn to_screen(origin: Pos2, page_rect: Rect, scale: f32, x: f32, y: f32) -> Pos2 {
    pos2(origin.x + page_rect.min.x + x * scale, origin.y + page_rect.min.y + y * scale)
}

/// Place a raster at a physical-pixel-aligned origin and its exact device-pixel size.
fn texel_aligned_rect(layout: Rect, texture_px: egui::Vec2, ppp: f32) -> Rect {
    let snap = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_size(pos2(snap(layout.min.x), snap(layout.min.y)), texture_px / ppp)
}

fn page_screen_scale(rect: Rect, page: &Page, fallback: f32) -> f32 {
    if page.w > 0.0 { rect.width() / page.w } else { fallback }
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let layout = app.session.layout();
    // View › Switch Modes: dark paper, inverted document colours, a white caret.
    let dark_page = app.session.view.dark_mode;
    let caret_color = if dark_page { Color32::WHITE } else { t.caret };
    let dark_paper = wordcraft_render::DARK_PAPER;
    let full = ui.available_rect_before_wrap();
    let show_ruler = app.session.view.ruler && app.session.view.mode == wordcraft_layout::ViewMode::Print && !app.session.view.read_mode;
    let (hruler, vruler, area) = if show_ruler {
        let h = Rect::from_min_max(pos2(full.min.x + RULER, full.min.y), pos2(full.max.x, full.min.y + RULER));
        let v = Rect::from_min_max(pos2(full.min.x, full.min.y + RULER), pos2(full.min.x + RULER, full.max.y));
        (Some(h), Some(v), Rect::from_min_max(pos2(full.min.x + RULER, full.min.y + RULER), full.max))
    } else {
        (None, None, full)
    };
    app.canvas.canvas_rect = Some(area);
    // Ctrl/⌘ + wheel (and trackpad pinch) zooms the document, like Word. egui reports both as a
    // zoom factor and doesn't scroll for them. Start from the shown scale so fit modes zoom too.
    let zoom_delta = ui.input(|i| i.zoom_delta());
    if (zoom_delta - 1.0).abs() > 1e-3 && ui.rect_contains_pointer(area) {
        let pct = (app.canvas.scale / PX_PER_PT * zoom_delta * 100.0).clamp(10.0, 500.0);
        let _ = app.run("view.zoom", json!({"value": pct}));
    }
    let geo = geometry(app, &layout, area.size() - vec2(14.0, 0.0));
    app.canvas.scale = geo.scale;
    // Zooming or resizing reflowed the pages into a different number of columns: the caret's page
    // moved, so bring it back into view rather than leave the reader somewhere else.
    if geo.cols != app.canvas.cols {
        app.canvas.cols = geo.cols;
        app.canvas.scroll_to_caret = true;
    }
    sync_fit_zoom(app, geo.scale);
    let caret = layout.caret_on(&app.session.sel.focus, app.session.page_hint);
    // Editing a header/footer (or a note) dims the body; once per frame, for every page.
    let dim_body = dims_body(app, &layout);
    if let Some(c) = caret {
        app.session.page_hint = c.page;
    }
    let mut scroll_target: Option<Rect> = None;
    if app.canvas.scroll_to_caret
        && let Some(c) = caret
        && let Some(pr) = geo.rects.get(c.page)
    {
        let r = Rect::from_min_size(pos2(pr.min.x + c.x * geo.scale, pr.min.y + c.top * geo.scale), vec2(2.0, c.height * geo.scale));
        scroll_target = Some(r.expand2(vec2(40.0, 60.0)));
    }
    app.canvas.scroll_to_caret = false;
    let mut origin = area.min;
    let mut ui_area = ui.new_child(egui::UiBuilder::new().max_rect(area));
    let out = egui::ScrollArea::both().id_salt("canvas_scroll").auto_shrink([false, false]).show_viewport(&mut ui_area, |ui, viewport| {
        origin = ui.min_rect().min - viewport.min.to_vec2();
        let content = Rect::from_min_size(ui.min_rect().min, geo.size);
        let resp = ui.allocate_rect(content, Sense::click_and_drag());
        if let Some(tr) = scroll_target {
            ui.scroll_to_rect(Rect::from_min_size(ui.min_rect().min + tr.min.to_vec2(), tr.size()), None);
        }
        let painter = ui.painter_at(ui.clip_rect());
        let ppp = ui.ctx().pixels_per_point();
        let max_tex = ui.ctx().input(|i| i.max_texture_side) as f32;
        let mut rendered = 0;
        let t0 = crate::now_ms();
        let mut rects = Vec::with_capacity(geo.rects.len());
        let content_origin = ui.min_rect().min;
        for (i, pr) in geo.rects.iter().enumerate() {
            let sr = pr.translate(content_origin.to_vec2());
            let Some(page) = layout.pages.get(i) else { continue };
            let scale_px = (geo.scale * ppp).min(max_tex / page.w.max(1.0)).min(max_tex / page.h.clamp(1.0, 1e6)).max(0.05);
            let key = page_key(app, page, scale_px, dim_body);
            let fresh = app.canvas.textures.get(&i).is_some_and(|(k, _)| *k == key);
            let current_scale = geo.scale * ppp;
            let visual_sr = if fresh && (scale_px - current_scale).abs() <= 0.0001_f32.max(current_scale.abs() * 0.0001) {
                app.canvas.textures.get(&i).map_or(sr, |(_, tex)| texel_aligned_rect(sr, tex.size_vec2(), ppp))
            } else {
                sr
            };
            rects.push(visual_sr);
            if !sr.intersects(ui.clip_rect().expand(200.0)) {
                continue;
            }
            // Shadow and paper.
            painter.rect_filled(visual_sr.translate(vec2(0.0, 2.0)).expand(1.5), 1.0, t.page_shadow);
            painter.rect_filled(visual_sr, 0.0, if dark_page { Color32::from_gray(dark_paper) } else { Color32::WHITE });
            if !fresh && (rendered < 2 || !app.canvas.textures.contains_key(&i) && rendered < 4) {
                let mut opts = screen_render_options();
                opts.display.marks = app.session.view.marks;
                opts.display.placeholders = true;
                opts.display.markup = app.session.view.show_markup;
                opts.dark = dark_page;
                opts.dark_paper = dark_paper;
                opts.display.dim_header = !dim_body;
                opts.display.dim_body = dim_body;
                let img = wordcraft_render::render_page(&app.session.doc, page, scale_px, &opts);
                let px = img.to_straight();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px);
                match app.canvas.textures.get_mut(&i) {
                    Some((k, tex)) => {
                        tex.set(ci, egui::TextureOptions::LINEAR);
                        *k = key;
                    }
                    None => {
                        let tex = ui.ctx().load_texture(format!("page{i}"), ci, egui::TextureOptions::LINEAR);
                        app.canvas.textures.insert(i, (key, tex));
                    }
                }
                rendered += 1;
            } else if !fresh {
                ui.ctx().request_repaint();
            }
            if let Some((_, tex)) = app.canvas.textures.get(&i) {
                painter.image(tex.id(), visual_sr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            let page_scale = page_screen_scale(visual_sr, page, geo.scale);
            // Header/footer editing chrome.
            if let StoryRef::Part(id) = app.session.sel.focus.story {
                let sect_body = page.body;
                if page.header_story == Some(id) || page.footer_story == Some(id) {
                    let hy = visual_sr.min.y + sect_body.y * page_scale - 4.0;
                    let fy = visual_sr.min.y + sect_body.bottom() * page_scale + 4.0;
                    for (y, label) in [(hy, "Header"), (fy, "Footer")] {
                        dashed(&painter, pos2(visual_sr.min.x, y), pos2(visual_sr.max.x, y), Stroke::new(1.0, t.accent));
                        let tr = Rect::from_min_size(pos2(visual_sr.min.x + 2.0, if label == "Header" { y } else { y - 18.0 }), vec2(52.0, 18.0));
                        painter.rect_filled(tr, 2.0, t.checked);
                        painter.text(tr.center(), egui::Align2::CENTER_CENTER, tl!(label), regular(11.0), t.accent_text);
                    }
                }
            }
            // Table gridlines.
            if app.session.view.gridlines {
                for it in &page.items {
                    if let Placed::Cell { rect, .. } = it {
                        let r = Rect::from_min_size(
                            pos2(visual_sr.min.x + rect.x * page_scale, visual_sr.min.y + rect.y * page_scale),
                            vec2(rect.w * page_scale, rect.h * page_scale),
                        );
                        painter.rect_stroke(r, 0.0, Stroke::new(0.5, t.blue.linear_multiply(0.6)), egui::StrokeKind::Middle);
                    }
                }
            }
        }
        if rendered > 0 {
            app.canvas.render_ms = crate::now_ms() - t0;
        }
        // Drop textures of pages that no longer exist.
        let n = layout.pages.len();
        app.canvas.textures.retain(|k, _| *k < n);
        app.canvas.page_rects = rects.clone();
        balloons(app, ui, &painter, &rects, &layout, geo.scale);
        // Selection (a selected object shows its frame instead).
        if !app.session.sel.is_collapsed() && crate::objects::selected(app).is_none() {
            let (a, b) = app.session.sel.ordered();
            let mut first: Option<Rect> = None;
            for (pi, r) in layout.selection_rects(&app.session.doc, &a, &b, app.session.page_hint) {
                if let Some(pr) = rects.get(pi) {
                    let scale = layout.pages.get(pi).map_or(geo.scale, |page| page_screen_scale(*pr, page, geo.scale));
                    let sr = Rect::from_min_size(pos2(pr.min.x + r.x * scale, pr.min.y + r.y * scale), vec2(r.w * scale, r.h * scale));
                    painter.rect_filled(sr, 0.0, t.selection);
                    // Track the topmost rect: the mini toolbar anchors on the selection's first line.
                    if first.is_none_or(|f| sr.min.y < f.min.y) {
                        first = Some(sr);
                    }
                }
            }
            app.canvas.mini_anchor = first;
        } else {
            app.canvas.mini_anchor = None;
        }
        // Read Aloud: the sentence being spoken.
        if let Some((a, b)) = crate::read_aloud::highlight(app) {
            for (pi, r) in layout.selection_rects(&app.session.doc, &a, &b, app.session.page_hint) {
                if let Some(pr) = rects.get(pi) {
                    let sr =
                        Rect::from_min_size(pos2(pr.min.x + r.x * geo.scale, pr.min.y + r.y * geo.scale), vec2(r.w * geo.scale, r.h * geo.scale));
                    painter.rect_filled(sr, 0.0, t.accent.gamma_multiply(0.16));
                    painter.hline(sr.x_range(), sr.max.y - 0.5, egui::Stroke::new(1.5, t.accent));
                }
            }
        }
        // Caret.
        let focused = resp.has_focus() || app.canvas.focused;
        // Editing an equation: shade it and draw the caret inside it.
        let mut math_caret = false;
        if let Some(m) = app.session.math.clone()
            && let Some((pi, ex, base, ml)) = layout.equation_geom(&m.at, app.session.page_hint)
            && let Some(pr) = rects.get(pi)
        {
            math_caret = true;
            let s = geo.scale;
            let zone = Rect::from_min_max(
                pos2(pr.min.x + ex * s - 2.0, pr.min.y + (base - ml.ascent) * s - 2.0),
                pos2(pr.min.x + (ex + ml.width) * s + 2.0, pr.min.y + (base + ml.descent) * s + 2.0),
            );
            painter.rect_filled(zone, 2.0, Color32::from_black_alpha(16));
            if let Some(slot) = ml.slot(&m.pos.path, m.pos.off) {
                let x = (pr.min.x + (ex + slot.x) * s).round() + 0.5;
                let y0 = pr.min.y + (base - slot.y - slot.a) * s;
                let y1 = pr.min.y + (base - slot.y + slot.d) * s;
                let since = crate::now_ms() - app.canvas.caret_visible_since;
                if ((since / 530.0) as u64).is_multiple_of(2) && focused {
                    painter.line_segment([pos2(x, y0), pos2(x, y1)], Stroke::new(1.5, t.caret));
                }
                if focused {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(530 - (since as u64 % 530)));
                    let cr = Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1));
                    ui.ctx().output_mut(|o| {
                        o.ime = Some(egui::output::IMEOutput {
                            rect: cr,
                            cursor_rect: cr,
                            purpose: Default::default(),
                            should_interrupt_composition: false,
                        });
                    });
                }
            }
        }
        if !math_caret
            && let Some(c) = layout.caret_on(&app.session.sel.focus, app.session.page_hint)
            && let Some(pr) = rects.get(c.page)
        {
            let scale = layout.pages.get(c.page).map_or(geo.scale, |page| page_screen_scale(*pr, page, geo.scale));
            let x = pr.min.x + c.x * scale;
            let y0 = pr.min.y + c.top * scale;
            let y1 = y0 + c.height * scale;
            let since = crate::now_ms() - app.canvas.caret_visible_since;
            let on = ((since / 530.0) as u64).is_multiple_of(2);
            if app.session.sel.is_collapsed() && on && focused {
                painter.line_segment([pos2(x.round() + 0.5, y0), pos2(x.round() + 0.5, y1)], Stroke::new(1.5, caret_color));
            }
            if focused {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(530 - (since as u64 % 530)));
            }
            // IME candidate window placement.
            let cr = Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1));
            if focused {
                ui.ctx().output_mut(|o| {
                    o.ime =
                        Some(egui::output::IMEOutput { rect: cr, cursor_rect: cr, purpose: Default::default(), should_interrupt_composition: false });
                });
            }
            if !app.canvas.ime_preedit.is_empty() {
                let g = painter.text(pos2(x, y1), egui::Align2::LEFT_BOTTOM, &app.canvas.ime_preedit, regular(c.height * scale * 0.8), caret_color);
                painter.line_segment([pos2(g.min.x, g.max.y), pos2(g.max.x, g.max.y)], Stroke::new(1.0, caret_color));
            }
        }
        crate::objects::paint(app, &painter, &t, &layout, &rects, geo.scale);
        (resp, rects)
    });
    let (resp, rects) = out.inner;
    // Focus: the canvas takes keyboard focus on click and keeps Tab/arrows.
    if resp.clicked() || resp.drag_started() || app.canvas.want_focus {
        resp.request_focus();
        app.canvas.want_focus = false;
    }
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
        });
    }
    app.canvas.focused = resp.has_focus();
    mouse(app, ui, &resp, &rects, &layout, geo.scale);
    // Right-click: move the caret there (unless inside the selection), then the context menu.
    // Right-click in an equation puts the caret there and opens the equation menu.
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((page, x, y)) = page_at(&rects, &layout, geo.scale, p)
    {
        match layout.equation_hit(page, x, y, app.session.sel.focus.story) {
            Some((at, inner)) => {
                app.session.sel = wordcraft_engine::Selection::caret(at);
                let _ = app.run("equation.edit", json!({"pos": serde_json::to_value(&inner).unwrap_or_default()}));
            }
            None if app.session.math.is_some() => {
                let _ = app.run("equation.exit", json!({}));
            }
            None => {}
        }
    }
    if resp.secondary_clicked()
        && app.session.math.is_none()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((page, x, y)) = page_at(&rects, &layout, geo.scale, p)
        && let Some(pos) = layout.hit(page, x, y, app.session.sel.focus.story)
    {
        let (a, b) = app.session.sel.ordered();
        if !(a <= pos && pos <= b) || app.session.sel.is_collapsed() {
            app.session.sel = wordcraft_engine::Selection::caret(pos);
        }
        app.canvas.context_issue = app.session.run("review.suggestions", &json!({})).ok().filter(|v| !v.is_null());
        app.canvas.context_synonyms = app.session.run("review.thesaurus", &json!({})).ok().and_then(|v| v.get("synonyms").cloned());
    }
    resp.context_menu(|ui| context_menu(app, ui));
    // Stand down while the context menu owns the pointer so the two never double up;
    // `context_menu_opened` also covers the click that dismisses the menu.
    app.canvas.context_menu_open = resp.context_menu_opened();
    if resp.hovered() || app.canvas.obj_drag.is_some() {
        let over_object = ui.input(|i| i.pointer.latest_pos()).and_then(|p| crate::objects::cursor(app, &layout, &rects, geo.scale, p));
        ui.ctx().set_cursor_icon(over_object.unwrap_or(egui::CursorIcon::Text));
    }
    // While "Save changes?" is up, keys answer it rather than edit the document behind it.
    if app.canvas.focused && !matches!(app.dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })) {
        crate::keys::canvas_events(app, ui.ctx());
    }
    let _ = origin;
    if let (Some(h), Some(v)) = (hruler, vruler) {
        rulers(app, ui, h, v, &rects, &layout, geo.scale);
    }
    if let Some(sel) = app.canvas.mini_anchor {
        crate::mini_toolbar::show(app, ui.ctx(), sel, area);
    }
}

/// Comment balloons in the markup area right of each page, joined to their anchors.
fn balloons(app: &mut WordApp, ui: &mut Ui, painter: &egui::Painter, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let mw = markup_width(app);
    if mw <= 0.0 {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let clip = ui.clip_rect();
    // (page, anchor x, anchor y, id) for each anchored, unresolved-or-not comment.
    let mut by_page: HashMap<usize, Vec<(f32, f32, u32, Pos)>> = HashMap::new();
    for (id, pos) in wordcraft_engine::cmd::review::comment_list(&app.session) {
        let Some(pos) = pos else { continue };
        let Some(c) = layout.caret_on(&pos, app.session.page_hint) else { continue };
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, id, pos));
    }
    let palette = [t.blue, Color32::from_rgb(0xB0, 0x3A, 0x2E), Color32::from_rgb(0x2E, 0x7D, 0x32), Color32::from_rgb(0x8E, 0x44, 0xAD), t.orange];
    let mut authors: Vec<String> = Vec::new();
    let mut clicked: Option<Pos> = None;
    // The markup area extends each page.
    for (pi, pr) in rects.iter().enumerate() {
        let page_scale = layout.pages.get(pi).map_or(scale, |page| page_screen_scale(*pr, page, scale));
        let area = Rect::from_min_max(pos2(pr.max.x, pr.min.y), pos2(pr.max.x + mw * page_scale, pr.max.y));
        if area.intersects(clip) {
            painter.rect_filled(area, 0.0, Color32::from_rgb(0xF3, 0xF3, 0xF3));
            painter.line_segment([area.left_top(), area.left_bottom()], Stroke::new(1.0, Color32::from_rgb(0xE0, 0xE0, 0xE0)));
        }
    }
    for (pi, list) in by_page.iter_mut() {
        let Some(pr) = rects.get(*pi) else { continue };
        if !pr.expand(400.0).intersects(clip) {
            continue;
        }
        list.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        let page_scale = layout.pages.get(*pi).map_or(scale, |page| page_screen_scale(*pr, page, scale));
        let x0 = pr.max.x + 10.0;
        let w = (mw * page_scale - 20.0).max(60.0);
        let mut next_y = pr.min.y;
        for (ax, ay, id, pos) in list.iter() {
            let Some(c) = app.session.doc.comments.get(id) else { continue };
            let text = app.session.doc.plain_text(StoryRef::Part(c.part));
            let author = if c.author.is_empty() { tl!("Author").to_string() } else { c.author.clone() };
            let ai = authors.iter().position(|a| *a == author).unwrap_or_else(|| {
                authors.push(author.clone());
                authors.len() - 1
            });
            let color = palette.get(ai % palette.len()).copied().unwrap_or(t.blue);
            let fs = (11.0 * page_scale / PX_PER_PT).clamp(8.0, 16.0);
            let head = painter.layout(author.clone(), semibold(fs), t.text, w - 16.0);
            let body = painter.layout(text.trim().to_string(), regular(fs), if c.resolved { t.text_dim } else { t.text }, w - 16.0);
            let h = head.size().y + body.size().y + 16.0;
            let anchor = pos2(pr.min.x + ax * page_scale, pr.min.y + ay * page_scale);
            let top = (anchor.y - 12.0).max(next_y);
            let card = Rect::from_min_size(pos2(x0, top), vec2(w, h));
            next_y = card.max.y + 6.0;
            if !card.intersects(clip) {
                continue;
            }
            let selected = app.session.sel.focus.path == pos.path && app.session.sel.focus.story == pos.story;
            // Leader: from the anchor along the text to the page edge, then to the card.
            let lead = Stroke::new(if selected { 1.5 } else { 1.0 }, color.linear_multiply(if selected { 1.0 } else { 0.7 }));
            dashed(painter, anchor, pos2(pr.max.x, anchor.y), lead);
            painter.line_segment([pos2(pr.max.x, anchor.y), pos2(x0, top + 10.0)], lead);
            painter.rect_filled(card.translate(vec2(0.0, 1.0)), 4.0, t.page_shadow);
            painter.rect_filled(card, 4.0, if selected { t.checked } else { Color32::WHITE });
            painter.rect_stroke(card, 4.0, Stroke::new(if selected { 1.5 } else { 1.0 }, color), egui::StrokeKind::Inside);
            painter.rect_filled(Rect::from_min_size(card.min, vec2(4.0, h)), egui::CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, color);
            painter.galley(pos2(card.min.x + 10.0, card.min.y + 6.0), head.clone(), t.text);
            painter.galley(pos2(card.min.x + 10.0, card.min.y + 10.0 + head.size().y), body, t.text);
            let r = ui.interact(card, ui.id().with(("balloon", *id)), Sense::click());
            if r.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked() {
                clicked = Some(pos.clone());
            }
        }
    }
    if let Some(p) = clicked {
        app.session.sel = wordcraft_engine::Selection::caret(p);
        app.canvas.want_focus = true;
    }
}

fn dashed(p: &egui::Painter, a: Pos2, b: Pos2, s: Stroke) {
    p.extend(egui::Shape::dashed_line(&[a, b], s, 4.0, 3.0));
}

/// Page index and document coordinates under a screen point.
pub fn page_at(rects: &[Rect], layout: &DocLayout, scale: f32, p: Pos2) -> Option<(usize, f32, f32)> {
    let i = nearest_page(rects, p)?;
    let r = rects.get(i)?;
    let scale = layout.pages.get(i).map_or(scale, |page| page_screen_scale(*r, page, scale));
    Some((i, (p.x - r.min.x) / scale, (p.y - r.min.y) / scale))
}

/// The page whose screen rect is under (or nearest to) a screen point.
pub fn nearest_page(rects: &[Rect], p: Pos2) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, r) in rects.iter().enumerate() {
        let d = if r.contains(p) {
            0.0
        } else {
            let dx = (r.min.x - p.x).max(p.x - r.max.x).max(0.0);
            let dy = (r.min.y - p.y).max(p.y - r.max.y).max(0.0);
            dx + dy * 2.0
        };
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

fn mouse(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let pointer = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
    // An object drag follows the pointer anywhere until it's released.
    let object_pointer = pointer.or_else(|| app.canvas.obj_drag.as_ref().and_then(|_| ui.input(|i| i.pointer.latest_pos())));
    if let Some(at) = object_pointer
        && crate::objects::pointer(app, ui, resp, rects, layout, scale, at)
    {
        return;
    }
    let Some(p) = pointer else { return };
    let Some((page, x, y)) = page_at(rects, layout, scale, p) else { return };
    let mods = ui.input(|i| i.modifiers);
    let story = app.session.sel.focus.story;
    // Inside an equation, clicks place the caret in it (and double-clicks don't select words).
    if (resp.double_clicked() || resp.triple_clicked()) && layout.equation_hit(page, x, y, story).is_some() {
        return;
    }
    // Double-click in the header/footer area edits it; double-click in the body leaves it.
    if resp.double_clicked() {
        if let Some((s, header)) = layout.header_footer_at(page, y)
            && story == StoryRef::Body
        {
            let id = if header { "insert.editHeader" } else { "insert.editFooter" };
            let _ = app.run(id, json!({}));
            let _ = s;
            if let Some(pos) = app.session.layout().hit(page, x, y, app.session.sel.focus.story) {
                app.session.sel = wordcraft_engine::Selection::caret(pos);
            }
            return;
        }
        if matches!(story, StoryRef::Part(_)) && !in_text_box(app) && layout.header_footer_at(page, y).is_none() {
            let _ = app.run("insert.closeHeader", json!({}));
            if let Some(pos) = layout.hit(page, x, y, StoryRef::Body) {
                app.session.sel = wordcraft_engine::Selection::caret(pos);
            }
            return;
        }
        let _ = app.run("select.word", json!({}));
        return;
    }
    if resp.triple_clicked() {
        let _ = app.run("select.paragraph", json!({}));
        return;
    }
    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer();
    if pressed {
        // Clicking into a footnote/endnote or a text box edits it; clicking outside a text box goes
        // back to where it is. While editing a header/footer, only its own text boxes are in reach.
        let story = {
            let is_note = |s: StoryRef| matches!(part_kind(app, s), Some(PartKind::Footnote | PartKind::Endnote));
            let is_box = |s: StoryRef| part_kind(app, s) == Some(PartKind::TextBox);
            let host = if is_box(story) { text_box_host(app, layout).unwrap_or(StoryRef::Body) } else { story };
            let in_hf = matches!(part_kind(app, host), Some(PartKind::Header | PartKind::Footer));
            let hf_box = if in_hf { layout.header_footer_text_box_at(page, x, y).map(StoryRef::Part) } else { None };
            match (hf_box, layout.story_at(page, x, y)) {
                (Some(b), _) => b,
                (None, Some(s)) if is_note(s) => s,
                (None, Some(s)) if is_box(s) && !in_hf => s,
                (None, Some(StoryRef::Body)) if is_note(story) => StoryRef::Body,
                _ if is_box(story) => host,
                _ => story,
            }
        };
        if !mods.shift
            && let Some((at, inner)) = layout.equation_hit(page, x, y, story)
        {
            app.session.sel = wordcraft_engine::Selection::caret(at);
            let _ = app.run("equation.edit", json!({"pos": serde_json::to_value(&inner).unwrap_or_default()}));
            app.session.page_hint = page;
            return;
        }
        if app.session.math.is_some() {
            let _ = app.run("equation.exit", json!({}));
        }
        let Some(pos) = layout.hit(page, x, y, story) else { return };
        // Ctrl/⌘+click follows a hyperlink.
        if mods.command
            && let Some(link) = app.session.doc.para_at(&pos).and_then(|pp| pp.props_of_char(pos.off).link.clone())
        {
            if let Some(name) = link.strip_prefix('#') {
                let _ = app.run("edit.goto", json!({"bookmark": name}));
            } else if is_followable_link(&link) {
                app.canvas.open_url = Some(link);
            } else {
                app.status(tl!("Only web (http, https) and email (mailto) links can be followed"));
            }
            return;
        }
        let pj = serde_json::to_value(&pos).unwrap_or_default();
        let _ = app.run("caret.set", json!({"pos": pj, "extend": mods.shift}));
        if mods.command && !mods.shift {
            let _ = app.run("select.sentence", json!({}));
        }
        app.session.page_hint = page;
        app.canvas.dragging = true;
        // Format painter applies on mouse up.
    } else if app.canvas.dragging && ui.input(|i| i.pointer.primary_down()) {
        if let Some(pos) = layout.hit(page, x, y, story)
            && pos != app.session.sel.focus
        {
            app.session.sel.focus = pos;
            app.session.page_hint = page;
            app.canvas.caret_visible_since = crate::now_ms();
        }
        // Auto-scroll near the edges is handled by egui's scroll area drag.
    } else if app.canvas.dragging && ui.input(|i| i.pointer.primary_released()) {
        app.canvas.dragging = false;
        if app.session.painter.is_some() && !app.session.sel.is_collapsed() {
            let _ = app.run("edit.pasteFormat", json!({}));
        }
    }
    // Hover a link: show its target.
    if resp.hovered()
        && !app.canvas.dragging
        && let Some(pos) = layout.hit(page, x, y, story)
        && let Some(link) = app.session.doc.para_at(&pos).and_then(|pp| pp.props_of_char(pos.off).link.clone())
    {
        let key = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
        let tip = format!("{link}\n{}", crate::i18n::fmt(tl!("{key}+Click to follow link"), &[("key", key)]));
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::new("link_tip"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(tip);
        });
    }
}

/// Horizontal and vertical rulers for the caret's page and paragraph.
fn rulers(app: &mut WordApp, ui: &mut Ui, h: Rect, v: Rect, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    p.rect_filled(Rect::from_min_max(pos2(v.min.x, h.min.y), pos2(v.max.x, h.max.y)), 0.0, t.canvas);
    p.rect_filled(h, 0.0, t.canvas);
    p.rect_filled(v, 0.0, t.canvas);
    let pi = app.session.page_hint.min(layout.pages.len().saturating_sub(1));
    let (Some(page), Some(pr)) = (layout.pages.get(pi), rects.get(pi)) else { return };
    let scale = page_screen_scale(*pr, page, scale);
    let sect = app.session.doc.sections().get(page.section).map(|(_, s)| (*s).clone()).unwrap_or_default();
    // Horizontal.
    let hp = ui.painter_at(h);
    let x0 = pr.min.x;
    let bar = Rect::from_min_max(pos2(x0, h.min.y + 4.0), pos2(x0 + page.w * scale, h.max.y - 3.0));
    hp.rect_filled(bar, 0.0, t.ruler_margin);
    let text = Rect::from_min_max(pos2(x0 + page.body.x * scale, bar.min.y), pos2(x0 + page.body.right() * scale, bar.max.y));
    hp.rect_filled(text, 0.0, t.ruler);
    let unit = 72.0;
    let origin = page.body.x;
    let mut k = -((origin / unit).ceil() as i32) * 8;
    loop {
        let xpt = origin + k as f32 * unit / 8.0;
        if xpt > page.w {
            break;
        }
        if xpt >= 0.0 {
            let sx = x0 + xpt * scale;
            let (len, label) = if k % 8 == 0 {
                (0.0, Some(k / 8))
            } else if k % 4 == 0 {
                (5.0, None)
            } else if k % 2 == 0 {
                (3.0, None)
            } else {
                (1.5, None)
            };
            match label {
                Some(n) if n != 0 => {
                    hp.text(pos2(sx, bar.center().y), egui::Align2::CENTER_CENTER, n.abs().to_string(), regular(9.5), t.ruler_tick);
                }
                Some(_) => {}
                None => {
                    if scale * unit / 8.0 > 4.0 || k % 2 == 0 {
                        hp.line_segment([pos2(sx, bar.center().y - len / 2.0), pos2(sx, bar.center().y + len / 2.0)], Stroke::new(1.0, t.ruler_tick));
                    }
                }
            }
        }
        k += 1;
        if k > 400 {
            break;
        }
    }
    // Indent markers for the caret's paragraph (relative to its column).
    let col_x = page.body.x;
    if let Some(para) = app.session.doc.para_at(&app.session.sel.focus) {
        let rp = app.session.doc.styles.resolve_para(&para.props);
        // Indents and tabs are measured from the paragraph's start edge: the right margin of a
        // right-to-left paragraph, where the start-indent markers sit.
        let rtl = rp.bidi;
        let w = page.body.w;
        let sx = |s: f32| if rtl { x0 + (col_x + w - s) * scale } else { x0 + (col_x + s) * scale };
        let s_at = |x: f32| if rtl { col_x + w - (x - x0) / scale } else { (x - x0) / scale - col_x };
        let first = sx(rp.indent_left + rp.indent_first);
        let left = sx(rp.indent_left);
        let right = sx(w - rp.indent_right);
        let c = t.text_dim;
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(first - 4.5, bar.min.y), pos2(first + 4.5, bar.min.y), pos2(first, bar.min.y + 5.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(left - 4.5, bar.max.y - 3.0), pos2(left + 4.5, bar.max.y - 3.0), pos2(left, bar.max.y - 8.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        hp.rect(
            Rect::from_min_max(pos2(left - 4.5, bar.max.y - 3.0), pos2(left + 4.5, bar.max.y + 1.0)),
            0.0,
            t.ruler,
            Stroke::new(1.0, c),
            egui::StrokeKind::Inside,
        );
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(right - 4.5, bar.max.y - 1.0), pos2(right + 4.5, bar.max.y - 1.0), pos2(right, bar.max.y - 6.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        for tab in &rp.tabs {
            let tx = sx(tab.pos);
            let foot = if rtl { -4.0 } else { 4.0 };
            hp.line_segment([pos2(tx, bar.max.y - 6.0), pos2(tx, bar.max.y - 1.0)], Stroke::new(1.5, t.text));
            hp.line_segment([pos2(tx, bar.max.y - 1.0), pos2(tx + foot, bar.max.y - 1.0)], Stroke::new(1.5, t.text));
        }
        // Dragging the left-indent marker.
        let id = ui.id().with("ruler_left");
        let mr = Rect::from_center_size(pos2(left, bar.max.y - 3.0), vec2(12.0, 12.0));
        let r = ui.interact(mr, id, Sense::drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = s_at(pp.x).clamp(-col_x, page.body.w - 18.0);
            let snapped = (pt / 4.5).round() * 4.5;
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"left": snapped}));
        }
        let fr = Rect::from_center_size(pos2(first, bar.min.y + 3.0), vec2(12.0, 10.0));
        let r = ui.interact(fr, ui.id().with("ruler_first"), Sense::drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = s_at(pp.x) - rp.indent_left;
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"firstLine": (pt / 4.5).round() * 4.5}));
        }
        let rr = Rect::from_center_size(pos2(right, bar.max.y - 3.0), vec2(12.0, 12.0));
        let r = ui.interact(rr, ui.id().with("ruler_right"), Sense::drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = page.body.w - s_at(pp.x);
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"right": (pt / 4.5).round() * 4.5}));
        }
    }
    // Vertical.
    let vp = ui.painter_at(v);
    let y0 = pr.min.y;
    let vbar = Rect::from_min_max(pos2(v.min.x + 4.0, y0), pos2(v.max.x - 3.0, y0 + page.h.min(20_000.0) * scale));
    vp.rect_filled(vbar, 0.0, t.ruler_margin);
    vp.rect_filled(Rect::from_min_max(pos2(vbar.min.x, y0 + page.body.y * scale), pos2(vbar.max.x, y0 + page.body.bottom() * scale)), 0.0, t.ruler);
    let origin = page.body.y;
    let mut k = -((origin / unit).ceil() as i32) * 8;
    loop {
        let ypt = origin + k as f32 * unit / 8.0;
        if ypt > page.h.min(20_000.0) {
            break;
        }
        if ypt >= 0.0 && k % 8 == 0 && k != 0 {
            vp.text(pos2(vbar.center().x, y0 + ypt * scale), egui::Align2::CENTER_CENTER, (k / 8).abs().to_string(), regular(9.5), t.ruler_tick);
        } else if ypt >= 0.0 && k % 4 == 0 {
            let sy = y0 + ypt * scale;
            vp.line_segment([pos2(vbar.center().x - 2.5, sy), pos2(vbar.center().x + 2.5, sy)], Stroke::new(1.0, t.ruler_tick));
        }
        k += 1;
        if k > 2000 {
            break;
        }
    }
    let _ = sect;
}

/// Convert a page position (points) to screen coordinates (for agents and tests).
pub fn page_to_screen(app: &mut WordApp, page: usize, x: f32, y: f32) -> Option<Pos2> {
    let r = app.canvas.page_rects.get(page)?;
    let layout = app.session.layout();
    let scale = layout.pages.get(page).map_or(app.canvas.scale, |page_data| page_screen_scale(*r, page_data, app.canvas.scale));
    Some(to_screen(pos2(0.0, 0.0), *r, scale, x, y))
}

/// Whether the caret is in a text box's story.
pub fn in_text_box(app: &WordApp) -> bool {
    part_kind(app, app.session.sel.focus.story) == Some(PartKind::TextBox)
}

fn part_kind(app: &WordApp, s: StoryRef) -> Option<PartKind> {
    match s {
        StoryRef::Part(id) => app.session.doc.parts.get(&id).map(|p| p.kind),
        StoryRef::Body => None,
    }
}

/// The story a text box's story sits in (where the box is), when the caret is in a text box.
fn text_box_host(app: &WordApp, layout: &DocLayout) -> Option<StoryRef> {
    match app.session.sel.focus.story {
        StoryRef::Part(id) if in_text_box(app) => layout.text_box(id, app.session.page_hint).map(|o| o.story),
        _ => None,
    }
}

/// Editing a header or footer, or a text box in one.
pub fn editing_header_footer(app: &WordApp, layout: &DocLayout) -> bool {
    let story = text_box_host(app, layout).unwrap_or(app.session.sel.focus.story);
    matches!(part_kind(app, story), Some(PartKind::Header | PartKind::Footer))
}

/// The body is dimmed while editing anything but the body (or a text box in it).
fn dims_body(app: &WordApp, layout: &DocLayout) -> bool {
    match app.session.sel.focus.story {
        StoryRef::Body => false,
        StoryRef::Part(_) if in_text_box(app) => editing_header_footer(app, layout),
        StoryRef::Part(_) => true,
    }
}

/// Caret position on screen.
pub fn caret_screen(app: &mut WordApp) -> Option<(Pos2, f32)> {
    let l = app.session.layout();
    let c = l.caret_on(&app.session.sel.focus, app.session.page_hint)?;
    let p = page_to_screen(app, c.page, c.x, c.top)?;
    let scale = app
        .canvas
        .page_rects
        .get(c.page)
        .and_then(|r| l.pages.get(c.page).map(|page| page_screen_scale(*r, page, app.canvas.scale)))
        .unwrap_or(app.canvas.scale);
    Some((p, c.height * scale))
}

pub fn pos_from_screen(app: &mut WordApp, p: Pos2) -> Option<Pos> {
    let layout = app.session.layout();
    let (page, x, y) = page_at(&app.canvas.page_rects, &layout, app.canvas.scale, p)?;
    let story = app.session.sel.focus.story;
    app.session.layout().hit(page, x, y, story)
}

/// Right-click inside an equation: its structures' actions, display and conversion.
fn equation_menu(app: &mut WordApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    let acts = app.session.run("equation.structureActions", &json!({})).ok().and_then(|v| v.get("actions").cloned()).unwrap_or_default();
    let mut last_level = None;
    for a in acts.as_array().into_iter().flatten() {
        let (Some(action), Some(label), Some(level)) =
            (a.get("action").and_then(|v| v.as_str()), a.get("label").and_then(|v| v.as_str()), a.get("level").and_then(|v| v.as_u64()))
        else {
            continue;
        };
        if last_level.is_some_and(|l| l != level) {
            ui.separator();
        }
        last_level = Some(level);
        if ui.button(tl!(label)).clicked() {
            let _ = app.run("equation.structure", json!({"action": action, "level": level}));
            app.canvas.want_focus = true;
            ui.close();
        }
    }
    if last_level.is_some() {
        ui.separator();
    }
    let display = app.session.run("equation.get", &json!({})).ok().and_then(|v| v.get("display").and_then(|d| d.as_bool())).unwrap_or(false);
    let items: [(&str, &str, serde_json::Value); 5] = [
        (if display { "Change to Inline" } else { "Change to Display" }, "equation.display", json!({"value": !display})),
        ("Professional", "equation.convert", json!({"to": "professional"})),
        ("Linear", "equation.convert", json!({"to": "linear"})),
        ("Equation Number", "equation.number", json!({})),
        ("Close Equation", "equation.exit", json!({})),
    ];
    for (label, id, params) in items {
        if ui.button(tl!(label)).clicked() {
            let _ = app.run(id, params);
            app.canvas.want_focus = true;
            ui.close();
        }
    }
}

fn context_menu(app: &mut WordApp, ui: &mut Ui) {
    if app.session.math.is_some() {
        return equation_menu(app, ui);
    }
    ui.set_min_width(220.0);
    let item = |ui: &mut Ui, app: &mut WordApp, label: &str, id: &str, params: serde_json::Value| {
        let sc = crate::widgets::shortcut_text(app, id);
        let on = crate::widgets::enabled(app, id);
        if ui.add_enabled(on, egui::Button::new(label).shortcut_text(sc)).clicked() {
            let _ = app.run(id, params);
            ui.close();
        }
    };
    if let Some(issue) = app.canvas.context_issue.clone() {
        let sugg: Vec<String> = issue
            .get("suggestions")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        ui.label(egui::RichText::new(issue.get("message").and_then(|m| m.as_str()).unwrap_or("")).small().weak());
        if sugg.is_empty() {
            ui.label(egui::RichText::new(tl!("(no suggestions)")).italics());
        }
        for sgt in sugg {
            if ui.button(egui::RichText::new(&sgt).strong()).clicked() {
                let _ = app.run("review.applySuggestion", json!({"text": sgt}));
                ui.close();
            }
        }
        if issue.get("kind").and_then(|k| k.as_str()) == Some("spelling") {
            item(ui, app, "Ignore All", "review.ignoreAll", json!({}));
            item(ui, app, "Add to Dictionary", "review.addToDictionary", json!({}));
        }
        ui.separator();
    }
    item(ui, app, "Cut", "edit.cut", json!({}));
    if ui.add(egui::Button::new(tl!("Copy")).shortcut_text(crate::widgets::shortcut_text(app, "edit.copy"))).clicked() {
        if let Ok(r) = app.run("edit.copy", json!({}))
            && let Some(t) = r.get("text").and_then(|t| t.as_str())
        {
            ui.ctx().copy_text(t.to_string());
        }
        ui.close();
    }
    item(ui, app, "Paste", "edit.paste", json!({}));
    ui.separator();
    if let Some(syn) = app.canvas.context_synonyms.clone().and_then(|v| v.as_array().cloned()).filter(|a| !a.is_empty()) {
        ui.menu_button(tl!("Synonyms"), |ui| {
            for w in syn.iter().filter_map(|x| x.as_str()) {
                if ui.button(w).clicked() {
                    let _ = app.run("select.word", json!({}));
                    let (a, b) = app.session.sel.ordered();
                    let trimmed = app.session.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off)).map(|t| t.trim_end().len()).unwrap_or(0);
                    let end = wordcraft_doc::Pos { off: a.off + trimmed, ..b };
                    app.session.sel = wordcraft_engine::Selection { anchor: a, focus: end };
                    let _ = app.run("text.insert", json!({"text": w, "raw": true}));
                    ui.close();
                }
            }
        });
    }
    item(ui, app, "Font…", "ui.dialog", json!({"name": "font"}));
    item(ui, app, "Paragraph…", "ui.dialog", json!({"name": "paragraph"}));
    item(ui, app, "Link…", "ui.dialog", json!({"name": "link"}));
    item(ui, app, "New Comment", "review.newComment", json!({}));
    if crate::ribbon::has_picture_selected(&app.session) {
        ui.separator();
        item(ui, app, "Change Picture…", "ui.changePicture", json!({}));
        item(ui, app, "Reset Picture", "picture.reset", json!({}));
        item(ui, app, "Size and Crop…", "ui.tab", json!({"tab": "Picture Format"}));
    }
    if app.session.sel.focus.path.cell().is_some() {
        ui.separator();
        ui.menu_button(tl!("Insert"), |ui| {
            item(ui, app, "Insert Rows Above", "table.insertRowAbove", json!({}));
            item(ui, app, "Insert Rows Below", "table.insertRowBelow", json!({}));
            item(ui, app, "Insert Columns to the Left", "table.insertColumnLeft", json!({}));
            item(ui, app, "Insert Columns to the Right", "table.insertColumnRight", json!({}));
        });
        ui.menu_button(tl!("Delete"), |ui| {
            item(ui, app, "Delete Rows", "table.deleteRow", json!({}));
            item(ui, app, "Delete Columns", "table.deleteColumn", json!({}));
            item(ui, app, "Delete Table", "table.deleteTable", json!({}));
        });
        item(ui, app, "Merge Cells", "table.merge", json!({}));
    }
}

/// Render options for page rasters shown on screen: on macOS, text is darkened the way the system
/// draws it, so a page looks as it does in other Mac apps (exports never are).
pub(crate) fn screen_render_options() -> wordcraft_render::RenderOptions {
    wordcraft_render::RenderOptions { text_darkening: cfg!(target_os = "macos"), ..Default::default() }
}

/// Whether Ctrl/⌘+click may hand a document's hyperlink to the system: web and email links only,
/// so a document can't launch `file:` paths, programs or custom-scheme handlers.
fn is_followable_link(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else { return false };
    ["http", "https", "mailto"].iter().any(|s| scheme.eq_ignore_ascii_case(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_email_links_are_followed() {
        for ok in ["https://example.com/a?b#c", "http://example.com", "HTTPS://EXAMPLE.COM", "mailto:someone@example.com"] {
            assert!(is_followable_link(ok), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "file://C:/Windows/System32/calc.exe",
            "C:\\Windows\\System32\\calc.exe",
            "/usr/bin/xterm",
            "javascript:alert(1)",
            "ms-msdt:/id",
            "smb://host/share",
            " https://example.com",
            "\thttps://example.com",
            "https\u{0}://example.com",
            "",
            "example.com",
        ] {
            assert!(!is_followable_link(bad), "{bad:?}");
        }
    }

    /// Issue #123: zoomed out, pages sit side by side in rows when the width allows; at 100% a
    /// letter page in an ordinary window stays one per row.
    #[test]
    fn pages_flow_into_rows_when_they_fit() {
        let letter = 612.0 * PX_PER_PT;
        assert_eq!(columns_for(1200.0, letter, 10, 1, true), 1, "100%: one page per row");
        assert_eq!(columns_for(1200.0, letter * 0.6, 10, 1, true), 2, "60%: two per row");
        assert_eq!(columns_for(1200.0, letter * 0.25, 10, 1, true), 5, "25%: five per row");
        assert_eq!(columns_for(1200.0, letter * 0.25, 3, 1, true), 3, "never more columns than pages");
        assert_eq!(columns_for(1200.0, letter * 0.25, 10, 1, false), 1, "fit modes that show one page");
        assert_eq!(columns_for(300.0, letter, 10, 2, true), 2, "Multiple Pages keeps two");
        // Hostile numbers never panic and give at least one column.
        for (w, slot, n) in
            [(f32::NAN, letter, 5), (1200.0, f32::NAN, 5), (f32::INFINITY, 1.0, 5), (1200.0, 0.0, 5), (-5.0, -1.0, 5), (1e30, 1e-30, 0)]
        {
            let c = columns_for(w, slot, n, 1, true);
            assert!((1..=n.max(1)).contains(&c), "{w} {slot} {n}: {c}");
        }

        // Mixed sizes: a landscape page among portrait ones, three per row.
        let sizes = [(612.0, 792.0), (792.0, 612.0), (612.0, 792.0), (612.0, 792.0)];
        let scale = 0.25;
        let (rects, size) = place_pages(&sizes, 0.0, scale, 3, vec2(1000.0, 600.0));
        assert_eq!(rects.len(), 4);
        assert_eq!(rects[0].top(), rects[1].top());
        assert_eq!(rects[1].top(), rects[2].top());
        assert!(rects[0].right() < rects[1].left() && rects[1].right() < rects[2].left(), "left to right: {rects:?}");
        assert!(rects[3].top() > rects[0].bottom(), "next row below the tallest page");
        assert_eq!(rects[3].left(), rects[0].left(), "rows share columns");
        assert!((rects[1].width() - 792.0 * scale).abs() < 1e-3);
        assert!(size.x >= 1000.0 && size.y >= rects[3].bottom());
        // Each page's centre maps back to that page.
        for (i, r) in rects.iter().enumerate() {
            assert_eq!(nearest_page(&rects, r.center()), Some(i));
        }
        // Hostile page sizes and scales stay finite.
        let bad = [(f32::NAN, f32::INFINITY), (-1.0, 0.0), (1e30, 1e30)];
        for scale in [f32::NAN, 0.0, -1.0, 1e9] {
            let (rects, size) = place_pages(&bad, f32::NAN, scale, 0, vec2(f32::NAN, -1.0));
            assert_eq!(rects.len(), 3);
            assert!(size.x.is_finite() && size.y.is_finite());
            assert!(rects.iter().all(|r| r.min.x.is_finite() && r.max.y.is_finite()), "{rects:?}");
        }
    }

    #[test]
    fn texel_aligned_rect_snaps_the_origin_and_sizes_from_texture_pixels() {
        let layout = Rect::from_min_size(pos2(10.25, 20.75), vec2(80.0, 120.0));
        let visual = texel_aligned_rect(layout, vec2(161.0, 241.0), 2.0);

        assert_eq!(visual.min, pos2(10.5, 21.0));
        assert_eq!(visual.size(), vec2(80.5, 120.5));
    }

    #[test]
    fn texel_aligned_rect_keeps_integer_pixel_pages_unchanged() {
        let layout = Rect::from_min_size(pos2(10.0, 20.0), vec2(80.0, 120.0));
        let visual = texel_aligned_rect(layout, vec2(160.0, 240.0), 2.0);

        assert_eq!(visual, layout);
    }
}

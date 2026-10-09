//! The document canvas: pages on a grey background, rendered by the engine into textures (one
//! per page, re-rendered only when the page's content changes), with the caret, selection,
//! rulers and mouse editing drawn by egui on top.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureHandle, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::{Pos, StoryRef};
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
        }
    }
}

/// Page positions in content space (points), for the current zoom.
pub struct Geometry {
    pub rects: Vec<Rect>,
    pub size: egui::Vec2,
    pub scale: f32,
}

/// Width of the markup area beside each page for comment balloons (points), or 0.
pub fn markup_width(app: &WordApp) -> f32 {
    let v = &app.session.view;
    // Word shows comments either in balloons (contextual) or in the Comments pane (list).
    let on = v.show_markup && !v.comments_pane && !v.read_mode && !v.multi_page && v.mode == wordcraft_layout::ViewMode::Print;
    if on && !app.session.doc.comments.is_empty() { 216.0 } else { 0.0 }
}

pub fn geometry(app: &WordApp, l: &DocLayout, avail: egui::Vec2) -> Geometry {
    let v = &app.session.view;
    let markup = markup_width(app);
    let maxw = l.pages.iter().map(|p| p.w).fold(0.0f32, f32::max).max(72.0) + markup;
    let maxh = l.pages.iter().map(|p| p.h.min(20_000.0)).fold(0.0f32, f32::max).max(72.0);
    let web = v.mode != wordcraft_layout::ViewMode::Print;
    let mut scale = v.zoom.clamp(0.1, 5.0) * PX_PER_PT;
    let cols = if v.multi_page || v.read_mode { 2 } else { 1 };
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
    let mut rects = Vec::with_capacity(l.pages.len());
    let row_w = cols as f32 * maxw * scale + (cols as f32 - 1.0) * GAP;
    let content_w = (row_w + 2.0 * GAP).max(avail.x);
    let mut y = GAP;
    for (i, chunk) in l.pages.chunks(cols).enumerate() {
        let _ = i;
        let h = chunk.iter().map(|p| p.h.min(1e6) * scale).fold(0.0f32, f32::max);
        let mut x = (content_w - row_w) / 2.0;
        for p in chunk {
            // Pages narrower than the widest keep their markup area beside them.
            let px = x + (maxw - markup - p.w).max(0.0) * scale / 2.0;
            rects.push(Rect::from_min_size(pos2(px, y), vec2(p.w * scale, p.h.min(1e6) * scale)));
            x += maxw * scale + GAP;
        }
        y += h + GAP;
    }
    Geometry { rects, size: vec2(content_w, y.max(avail.y)), scale }
}

/// Fingerprint of a page's content for the texture cache.
fn page_key(app: &WordApp, page: &Page, scale_px: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    scale_px.to_bits().hash(&mut h);
    let v = &app.session.view;
    (v.marks, v.show_markup).hash(&mut h);
    let editing_hf = matches!(app.session.sel.focus.story, StoryRef::Part(id) if Some(id) == page.header_story || Some(id) == page.footer_story);
    editing_hf.hash(&mut h);
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
                Placed::Cell { .. } => {}
            }
        }
    }
    app.session.doc.media.len().hash(&mut h);
    h.finish()
}

fn to_screen(origin: Pos2, page_rect: Rect, scale: f32, x: f32, y: f32) -> Pos2 {
    pos2(origin.x + page_rect.min.x + x * scale, origin.y + page_rect.min.y + y * scale)
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let layout = app.session.layout();
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
    let geo = geometry(app, &layout, area.size() - vec2(14.0, 0.0));
    app.canvas.scale = geo.scale;
    let caret = layout.caret_on(&app.session.sel.focus, app.session.page_hint);
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
            rects.push(sr);
            if !sr.intersects(ui.clip_rect().expand(200.0)) {
                continue;
            }
            let Some(page) = layout.pages.get(i) else { continue };
            // Shadow and paper.
            painter.rect_filled(sr.translate(vec2(0.0, 2.0)).expand(1.5), 1.0, t.page_shadow);
            painter.rect_filled(sr, 0.0, Color32::WHITE);
            let scale_px = (geo.scale * ppp).min(max_tex / page.w.max(1.0)).min(max_tex / page.h.clamp(1.0, 1e6)).max(0.05);
            let key = page_key(app, page, scale_px);
            let fresh = app.canvas.textures.get(&i).is_some_and(|(k, _)| *k == key);
            if !fresh && (rendered < 2 || !app.canvas.textures.contains_key(&i) && rendered < 4) {
                let mut opts = wordcraft_render::RenderOptions::default();
                opts.display.marks = app.session.view.marks;
                opts.display.markup = app.session.view.show_markup;
                let editing_hf = matches!(app.session.sel.focus.story, StoryRef::Part(_));
                opts.display.dim_header = !editing_hf;
                opts.display.dim_body = editing_hf;
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
                painter.image(tex.id(), sr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            // Header/footer editing chrome.
            if let StoryRef::Part(id) = app.session.sel.focus.story {
                let sect_body = page.body;
                if page.header_story == Some(id) || page.footer_story == Some(id) {
                    let hy = sr.min.y + sect_body.y * geo.scale - 4.0;
                    let fy = sr.min.y + sect_body.bottom() * geo.scale + 4.0;
                    for (y, label) in [(hy, "Header"), (fy, "Footer")] {
                        dashed(&painter, pos2(sr.min.x, y), pos2(sr.max.x, y), Stroke::new(1.0, t.accent));
                        let tr = Rect::from_min_size(pos2(sr.min.x + 2.0, if label == "Header" { y } else { y - 18.0 }), vec2(52.0, 18.0));
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
                            pos2(sr.min.x + rect.x * geo.scale, sr.min.y + rect.y * geo.scale),
                            vec2(rect.w * geo.scale, rect.h * geo.scale),
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
        // Selection.
        if !app.session.sel.is_collapsed() {
            let (a, b) = app.session.sel.ordered();
            for (pi, r) in layout.selection_rects(&app.session.doc, &a, &b, app.session.page_hint) {
                if let Some(pr) = rects.get(pi) {
                    let sr =
                        Rect::from_min_size(pos2(pr.min.x + r.x * geo.scale, pr.min.y + r.y * geo.scale), vec2(r.w * geo.scale, r.h * geo.scale));
                    painter.rect_filled(sr, 0.0, t.selection);
                }
            }
        }
        // Caret.
        let focused = resp.has_focus() || app.canvas.focused;
        if let Some(c) = layout.caret_on(&app.session.sel.focus, app.session.page_hint)
            && let Some(pr) = rects.get(c.page)
        {
            let x = pr.min.x + c.x * geo.scale;
            let y0 = pr.min.y + c.top * geo.scale;
            let y1 = y0 + c.height * geo.scale;
            let since = crate::now_ms() - app.canvas.caret_visible_since;
            let on = ((since / 530.0) as u64).is_multiple_of(2);
            if app.session.sel.is_collapsed() && on && focused {
                painter.line_segment([pos2(x.round() + 0.5, y0), pos2(x.round() + 0.5, y1)], Stroke::new(1.5, t.caret));
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
                let g = painter.text(pos2(x, y1), egui::Align2::LEFT_BOTTOM, &app.canvas.ime_preedit, regular(c.height * geo.scale * 0.8), t.caret);
                painter.line_segment([pos2(g.min.x, g.max.y), pos2(g.max.x, g.max.y)], Stroke::new(1.0, t.caret));
            }
        }
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
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((page, x, y)) = page_at(&rects, geo.scale, p)
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
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if app.canvas.focused {
        crate::keys::canvas_events(app, ui.ctx());
    }
    let _ = origin;
    if let (Some(h), Some(v)) = (hruler, vruler) {
        rulers(app, ui, h, v, &rects, &layout, geo.scale);
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
    for pr in rects {
        let area = Rect::from_min_max(pos2(pr.max.x, pr.min.y), pos2(pr.max.x + mw * scale, pr.max.y));
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
        let x0 = pr.max.x + 10.0;
        let w = (mw * scale - 20.0).max(60.0);
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
            let fs = (11.0 * scale / PX_PER_PT).clamp(8.0, 16.0);
            let head = painter.layout(author.clone(), semibold(fs), t.text, w - 16.0);
            let body = painter.layout(text.trim().to_string(), regular(fs), if c.resolved { t.text_dim } else { t.text }, w - 16.0);
            let h = head.size().y + body.size().y + 16.0;
            let anchor = pos2(pr.min.x + ax * scale, pr.min.y + ay * scale);
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
pub fn page_at(rects: &[Rect], scale: f32, p: Pos2) -> Option<(usize, f32, f32)> {
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
    let (i, _) = best?;
    let r = rects.get(i)?;
    Some((i, (p.x - r.min.x) / scale, (p.y - r.min.y) / scale))
}

fn mouse(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let Some(p) = resp.interact_pointer_pos().or_else(|| resp.hover_pos()) else { return };
    let Some((page, x, y)) = page_at(rects, scale, p) else { return };
    let mods = ui.input(|i| i.modifiers);
    let story = app.session.sel.focus.story;
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
        if matches!(story, StoryRef::Part(_)) && layout.header_footer_at(page, y).is_none() {
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
        // Clicking into a footnote/endnote edits it; clicking the body from a note goes back.
        let story = match layout.story_at(page, x, y) {
            Some(StoryRef::Part(id))
                if app
                    .session
                    .doc
                    .parts
                    .get(&id)
                    .is_some_and(|p| matches!(p.kind, wordcraft_doc::PartKind::Footnote | wordcraft_doc::PartKind::Endnote)) =>
            {
                StoryRef::Part(id)
            }
            Some(StoryRef::Body) if matches!(story, StoryRef::Part(id) if app.session.doc.parts.get(&id).is_some_and(|p| matches!(p.kind, wordcraft_doc::PartKind::Footnote | wordcraft_doc::PartKind::Endnote))) => {
                StoryRef::Body
            }
            _ => story,
        };
        let Some(pos) = layout.hit(page, x, y, story) else { return };
        // Ctrl/⌘+click follows a hyperlink.
        if mods.command
            && let Some(link) = app.session.doc.para_at(&pos).and_then(|pp| pp.props_of_char(pos.off).link.clone())
        {
            if let Some(name) = link.strip_prefix('#') {
                let _ = app.run("edit.goto", json!({"bookmark": name}));
            } else {
                app.canvas.open_url = Some(link);
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
        let first = x0 + (col_x + rp.indent_left + rp.indent_first) * scale;
        let left = x0 + (col_x + rp.indent_left) * scale;
        let right = x0 + (col_x + page.body.w - rp.indent_right) * scale;
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
            let tx = x0 + (col_x + tab.pos) * scale;
            hp.line_segment([pos2(tx, bar.max.y - 6.0), pos2(tx, bar.max.y - 1.0)], Stroke::new(1.5, t.text));
            hp.line_segment([pos2(tx, bar.max.y - 1.0), pos2(tx + 4.0, bar.max.y - 1.0)], Stroke::new(1.5, t.text));
        }
        // Dragging the left-indent marker.
        let id = ui.id().with("ruler_left");
        let mr = Rect::from_center_size(pos2(left, bar.max.y - 3.0), vec2(12.0, 12.0));
        let r = ui.interact(mr, id, Sense::drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = ((pp.x - x0) / scale - col_x).clamp(-col_x, page.body.w - 18.0);
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
            let pt = (pp.x - x0) / scale - col_x - rp.indent_left;
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
            let pt = page.body.w - ((pp.x - x0) / scale - col_x);
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
pub fn page_to_screen(app: &WordApp, page: usize, x: f32, y: f32) -> Option<Pos2> {
    let r = app.canvas.page_rects.get(page)?;
    Some(to_screen(pos2(0.0, 0.0), *r, app.canvas.scale, x, y))
}

/// Caret position on screen.
pub fn caret_screen(app: &mut WordApp) -> Option<(Pos2, f32)> {
    let l = app.session.layout();
    let c = l.caret_on(&app.session.sel.focus, app.session.page_hint)?;
    let p = page_to_screen(app, c.page, c.x, c.top)?;
    Some((p, c.height * app.canvas.scale))
}

pub fn pos_from_screen(app: &mut WordApp, p: Pos2) -> Option<Pos> {
    let (page, x, y) = page_at(&app.canvas.page_rects, app.canvas.scale, p)?;
    let story = app.session.sel.focus.story;
    app.session.layout().hit(page, x, y, story)
}

fn context_menu(app: &mut WordApp, ui: &mut Ui) {
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

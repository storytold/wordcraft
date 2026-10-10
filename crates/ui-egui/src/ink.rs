//! Drawing on the page with the Draw tab's pens, eraser and lasso, and Ink Replay. While a pen
//! is in use, a drag draws a live stroke and its release commits it with `draw.stroke` (one
//! command, one undo step); with the eraser, a click or drag over a stroke deletes it with
//! `draw.erase` (one undo step per drag). With the lasso, a drag draws a loop whose release
//! selects the ink inside (`draw.lasso`); dragging the selection moves it (`draw.lassoMove`),
//! Delete deletes it and Esc lets it go. Select (or Esc) goes back to ordinary selection.
//!
//! Ink Replay (`draw.replay`) hides the ink on the pages and draws the strokes again here, one
//! after another in the order they were drawn, with a bar of controls at the bottom.

use egui::{Align2, Color32, Key, Painter, Pos2, Rect, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::freeform::{InkTool, MAX_POINTS};
use wordcraft_engine::cmd::draw::DrawMode;
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::canvas::page_at;

/// A stroke being drawn: its page and points (page points).
pub struct InkDrag {
    page: usize,
    tool: InkTool,
    pts: Vec<[f32; 2]>,
}

/// The lasso in use: a loop being drawn, or the selection being dragged (page points).
pub enum LassoDrag {
    Loop { page: usize, pts: Vec<[f32; 2]> },
    Move { page: usize, from: [f32; 2], to: [f32; 2] },
}

/// Ink Replay in progress: the strokes in drawing order and how far it has got.
pub struct Replay {
    strokes: Vec<ReplayStroke>,
    /// Seconds into the replay.
    t: f32,
    playing: bool,
}

struct ReplayStroke {
    page: usize,
    paths: Vec<Vec<[f32; 2]>>,
    color: Color32,
    width: f32,
    /// When it starts being drawn and how long it takes, seconds.
    start: f32,
    dur: f32,
}

/// The Draw tab's pointer state beyond the pens (kept on the canvas).
#[derive(Default)]
pub struct InkUi {
    pub(crate) lasso: Option<LassoDrag>,
    pub(crate) replay: Option<Replay>,
}

/// Smallest move that adds a point to a live stroke, screen points.
const MIN_STEP_PX: f32 = 1.5;
/// How fast replayed ink is drawn, page points per second, and the pause between strokes.
const REPLAY_SPEED: f32 = 400.0;
const REPLAY_GAP: f32 = 0.12;
/// How close to the lasso selection's frame a press still grabs it, page points.
const GRAB_SLOP: f32 = 4.0;

/// Handle the pointer while a pen, the eraser or the lasso is in use; `true` when it did (no
/// text selection or object dragging then).
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) -> bool {
    let mode = app.session.view.draw.mode;
    if mode != DrawMode::Lasso {
        app.canvas.ink_ui.lasso = None;
        if app.session.view.draw.lasso.is_some() {
            let _ = app.run("draw.lasso", json!({"clear": true}));
        }
    }
    if mode == DrawMode::Select {
        app.canvas.ink = None;
        app.canvas.erasing = None;
        return false;
    }
    let (pressed, down, released, at) =
        ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.latest_pos()));
    let starts = pressed && resp.contains_pointer();
    match mode {
        DrawMode::Pen(tool) => {
            if starts && let Some((page, x, y)) = at.and_then(|p| page_at(rects, layout, scale, p)) {
                app.canvas.ink = Some(InkDrag { page, tool, pts: vec![[x, y]] });
            } else if down
                && let Some(p) = at
                && let Some(d) = app.canvas.ink.as_mut()
                && let Some((x, y)) = on_page(rects, layout, scale, d.page, p)
            {
                let far = d.pts.last().is_none_or(|[lx, ly]| ((x - lx).powi(2) + (y - ly).powi(2)).sqrt() * scale >= MIN_STEP_PX);
                if far && d.pts.len() < MAX_POINTS {
                    d.pts.push([x, y]);
                }
            }
            if released && let Some(d) = app.canvas.ink.take() {
                let _ = app.run("draw.stroke", json!({"page": d.page, "points": d.pts, "tool": d.tool.name()}));
            }
        }
        DrawMode::Eraser => {
            if starts {
                app.canvas.erasing = Some(false);
            }
            if (starts || down)
                && let Some(erased) = app.canvas.erasing
                && let Some((page, x, y)) = at.and_then(|p| page_at(rects, layout, scale, p))
                && wordcraft_engine::cmd::draw::ink_at(&mut app.session, page, x, y).is_some()
            {
                // The first stroke a drag erases starts an undo step; the rest join it.
                if erased {
                    app.session.join_next_undo();
                }
                if app.run("draw.erase", json!({"page": page, "x": x, "y": y})).is_ok() {
                    app.canvas.erasing = Some(true);
                }
            }
            if released {
                app.canvas.erasing = None;
            }
        }
        DrawMode::Lasso => lasso_pointer(app, (starts, down, released, at), rects, layout, scale),
        DrawMode::Select => {}
    }
    true
}

fn lasso_pointer(app: &mut WordApp, (starts, down, released, at): (bool, bool, bool, Option<Pos2>), rects: &[Rect], layout: &DocLayout, scale: f32) {
    if starts && let Some((page, x, y)) = at.and_then(|p| page_at(rects, layout, scale, p)) {
        // A press on the selection drags it; anywhere else starts a new loop.
        let grab = wordcraft_engine::cmd::draw::lasso_bounds(&app.session, layout).is_some_and(|(bp, r)| {
            bp == page && x >= r.x - GRAB_SLOP && x <= r.right() + GRAB_SLOP && y >= r.y - GRAB_SLOP && y <= r.bottom() + GRAB_SLOP
        });
        app.canvas.ink_ui.lasso =
            Some(if grab { LassoDrag::Move { page, from: [x, y], to: [x, y] } } else { LassoDrag::Loop { page, pts: vec![[x, y]] } });
    } else if down
        && let Some(p) = at
        && let Some(d) = app.canvas.ink_ui.lasso.as_mut()
    {
        match d {
            LassoDrag::Loop { page, pts } => {
                if let Some((x, y)) = on_page(rects, layout, scale, *page, p) {
                    let far = pts.last().is_none_or(|[lx, ly]| ((x - lx).powi(2) + (y - ly).powi(2)).sqrt() * scale >= MIN_STEP_PX * 2.0);
                    if far && pts.len() < MAX_POINTS {
                        pts.push([x, y]);
                    }
                }
            }
            LassoDrag::Move { page, to, .. } => {
                if let Some((x, y)) = on_page(rects, layout, scale, *page, p) {
                    *to = [x, y];
                }
            }
        }
    }
    if released && let Some(d) = app.canvas.ink_ui.lasso.take() {
        match d {
            LassoDrag::Loop { page, pts } if pts.len() >= 3 => {
                let _ = app.run("draw.lasso", json!({"page": page, "points": pts}));
            }
            // A click without a loop lets the selection go.
            LassoDrag::Loop { .. } => {
                let _ = app.run("draw.lasso", json!({"clear": true}));
            }
            LassoDrag::Move { from, to, .. } => {
                let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                if dx.abs() + dy.abs() > 0.25 {
                    let _ = app.run("draw.lassoMove", json!({"dx": dx, "dy": dy}));
                }
            }
        }
    }
}

/// Keys the lasso and Ink Replay take: Delete deletes the lasso selection, Esc lets it go or
/// stops the replay. `true` when the key was used.
pub fn key(app: &mut WordApp, key: Key) -> bool {
    if key == Key::Escape && app.session.view.draw.replay {
        stop_replay(app);
        return true;
    }
    let selected = wordcraft_engine::cmd::draw::lasso_selection(&app.session).is_some();
    match key {
        Key::Delete | Key::Backspace if selected => {
            let _ = app.run("draw.lassoDelete", json!({}));
            true
        }
        Key::Escape if selected => {
            let _ = app.run("draw.lasso", json!({"clear": true}));
            true
        }
        _ => false,
    }
}

/// Page point under screen point `p` on `page` (outside the page too, clamped by the command).
fn on_page(rects: &[Rect], layout: &DocLayout, scale: f32, page: usize, p: Pos2) -> Option<(f32, f32)> {
    let r = rects.get(page)?;
    let s = page_scale(rects, layout, scale, page)?;
    Some(((p.x - r.min.x) / s, (p.y - r.min.y) / s))
}

/// Screen points per page point on `page`.
fn page_scale(rects: &[Rect], layout: &DocLayout, scale: f32, page: usize) -> Option<f32> {
    let r = rects.get(page)?;
    let s = layout.pages.get(page).map_or(scale, |pg| crate::canvas::page_screen_scale(*r, pg, scale));
    (s > 0.0).then_some(s)
}

/// Page points to screen points on `page`.
fn to_screen(rects: &[Rect], layout: &DocLayout, scale: f32, page: usize) -> Option<impl Fn([f32; 2]) -> Pos2> {
    let r = *rects.get(page)?;
    let s = page_scale(rects, layout, scale, page)?;
    Some(move |[x, y]: [f32; 2]| pos2(r.min.x + x * s, r.min.y + y * s))
}

/// The live stroke, the lasso and its selection, and the replayed ink, drawn over the pages.
pub fn paint(app: &WordApp, painter: &Painter, rects: &[Rect], layout: &DocLayout, scale: f32) {
    paint_replay(app, painter, rects, layout, scale);
    paint_lasso(app, painter, rects, layout, scale);
    let Some(d) = &app.canvas.ink else { return };
    let Some(map) = to_screen(rects, layout, scale, d.page) else { return };
    let s = page_scale(rects, layout, scale, d.page).unwrap_or(scale);
    let set = app.session.view.draw.settings(d.tool);
    let alpha = (d.tool.alpha() * 255.0).round() as u8;
    let color = Color32::from_rgba_unmultiplied(set.color.0, set.color.1, set.color.2, alpha);
    let pts: Vec<Pos2> = d.pts.iter().map(|p| map(*p)).collect();
    let width = (set.width * s).max(1.0);
    if let [only] = pts.as_slice() {
        painter.circle_filled(*only, width / 2.0, color);
    } else {
        painter.add(egui::Shape::line(pts, Stroke::new(width, color)));
    }
}

fn paint_lasso(app: &WordApp, painter: &Painter, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let t = crate::theme::Tokens::get(painter.ctx());
    let dash = |pts: &[Pos2]| egui::Shape::dashed_line(pts, Stroke::new(1.0, t.accent), 4.0, 3.0);
    if let Some(LassoDrag::Loop { page, pts }) = &app.canvas.ink_ui.lasso
        && let Some(map) = to_screen(rects, layout, scale, *page)
    {
        let mut loop_pts: Vec<Pos2> = pts.iter().map(|p| map(*p)).collect();
        if let Some(first) = loop_pts.first().copied() {
            loop_pts.push(first);
        }
        painter.extend(dash(&loop_pts));
    }
    let Some((page, r)) = wordcraft_engine::cmd::draw::lasso_bounds(&app.session, layout) else { return };
    let (dx, dy) = match &app.canvas.ink_ui.lasso {
        Some(LassoDrag::Move { from, to, .. }) => (to[0] - from[0], to[1] - from[1]),
        _ => (0.0, 0.0),
    };
    let Some(map) = to_screen(rects, layout, scale, page) else { return };
    let (a, b) = (map([r.x + dx, r.y + dy]), map([r.right() + dx, r.bottom() + dy]));
    let sr = Rect::from_min_max(a, b).expand(3.0);
    painter.extend(dash(&[sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom(), sr.left_top()]));
}

/// The pointer over the page while a pen, the eraser or the lasso is in use.
pub fn cursor(app: &WordApp) -> Option<egui::CursorIcon> {
    match app.session.view.draw.mode {
        DrawMode::Select => None,
        DrawMode::Lasso if matches!(app.canvas.ink_ui.lasso, Some(LassoDrag::Move { .. })) => Some(egui::CursorIcon::Grabbing),
        DrawMode::Eraser | DrawMode::Pen(_) | DrawMode::Lasso => Some(egui::CursorIcon::Crosshair),
    }
}

// --- Ink Replay ----------------------------------------------------------------------------

/// Start, advance or end Ink Replay with the session's `draw.replay` switch; call once a frame
/// before the pages are drawn.
pub fn replay_tick(app: &mut WordApp, ctx: &egui::Context) {
    if !app.session.view.draw.replay {
        app.canvas.ink_ui.replay = None;
        return;
    }
    if app.canvas.ink_ui.replay.is_none() {
        let mut start = 0.0;
        let strokes: Vec<ReplayStroke> = wordcraft_engine::cmd::draw::strokes_in_drawing_order(&mut app.session)
            .into_iter()
            .map(|(page, k)| {
                let paths = k.paths();
                let len: f32 = paths.iter().map(|p| path_len(p)).sum();
                let dur = (len / REPLAY_SPEED).clamp(0.15, 2.5);
                let alpha = (k.freeform.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
                let c = k.color.unwrap_or_default();
                let width = if k.width.is_finite() { k.width.clamp(0.25, 72.0) } else { 1.0 };
                let st = ReplayStroke { page, paths, color: Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha), width, start, dur };
                start += dur + REPLAY_GAP;
                st
            })
            .collect();
        if strokes.is_empty() {
            stop_replay(app);
            return;
        }
        app.canvas.ink_ui.replay = Some(Replay { strokes, t: 0.0, playing: true });
    }
    let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
    if let Some(r) = app.canvas.ink_ui.replay.as_mut()
        && r.playing
    {
        r.t = (r.t + dt).min(r.end());
        if r.t >= r.end() {
            r.playing = false;
        }
        ctx.request_repaint();
    }
}

impl Replay {
    fn end(&self) -> f32 {
        self.strokes.last().map_or(0.0, |s| s.start + s.dur)
    }
    /// Strokes started so far.
    fn started(&self) -> usize {
        self.strokes.iter().filter(|s| s.start <= self.t).count()
    }
}

fn path_len(p: &[[f32; 2]]) -> f32 {
    p.windows(2).filter_map(|w| Some((w.first()?, w.get(1)?))).map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1])).sum()
}

/// The replayed ink so far: finished strokes whole, the current one up to where it has got.
fn paint_replay(app: &WordApp, painter: &Painter, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let Some(r) = &app.canvas.ink_ui.replay else { return };
    for k in r.strokes.iter().filter(|k| k.start <= r.t) {
        let Some(map) = to_screen(rects, layout, scale, k.page) else { continue };
        let s = page_scale(rects, layout, scale, k.page).unwrap_or(scale);
        let frac = if k.dur > 0.0 { ((r.t - k.start) / k.dur).clamp(0.0, 1.0) } else { 1.0 };
        let total: f32 = k.paths.iter().map(|p| path_len(p)).sum();
        let mut left = total * frac;
        let width = (k.width * s).max(1.0);
        for path in &k.paths {
            let mut pts: Vec<Pos2> = Vec::new();
            for w in path.windows(2) {
                let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
                if pts.is_empty() {
                    pts.push(map(*a));
                }
                let seg = (b[0] - a[0]).hypot(b[1] - a[1]);
                if seg <= left {
                    pts.push(map(*b));
                    left -= seg;
                } else {
                    let f = if seg > 0.0 { left / seg } else { 0.0 };
                    pts.push(map([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]));
                    left = 0.0;
                    break;
                }
            }
            match (pts.as_slice(), path.as_slice()) {
                (_, [only]) => {
                    painter.circle_filled(map(*only), width / 2.0, k.color);
                }
                ([], _) => {}
                _ => {
                    painter.add(egui::Shape::line(pts, Stroke::new(width, k.color)));
                }
            }
            if left <= 0.0 {
                break;
            }
        }
    }
}

fn stop_replay(app: &mut WordApp) {
    app.canvas.ink_ui.replay = None;
    let _ = app.run("draw.replay", json!({"value": false}));
}

/// Ink Replay's controls at the bottom of the document area: Rewind, Play/Pause, Forward (to
/// the end of the stroke being drawn), how far it has got, and Close.
pub fn replay_bar(app: &mut WordApp, ui: &Ui, area: Rect) {
    let Some(r) = app.canvas.ink_ui.replay.as_ref() else { return };
    let (playing, n, total) = (r.playing, r.started(), r.strokes.len());
    let mut action = None;
    egui::Area::new(egui::Id::new("ink_replay_bar"))
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(pos2(area.center().x, area.max.y - 18.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                    if crate::widgets::icon_button(ui, "replayRewind", tl!("Rewind")).clicked() {
                        action = Some("rewind");
                    }
                    let (icon, tip) = if playing { ("replayPause", tl!("Pause")) } else { ("replayPlay", tl!("Play")) };
                    if crate::widgets::icon_button(ui, icon, tip).clicked() {
                        action = Some("play");
                    }
                    if crate::widgets::icon_button(ui, "replayForward", tl!("Forward")).clicked() {
                        action = Some("forward");
                    }
                    let label = crate::i18n::fmt(tl!("Stroke {n} of {total}"), &[("n", &n.to_string()), ("total", &total.to_string())]);
                    ui.label(label);
                    if crate::widgets::icon_button(ui, "close", tl!("Close")).clicked() {
                        action = Some("close");
                    }
                });
            });
        });
    let Some(a) = action else { return };
    if a == "close" {
        stop_replay(app);
        return;
    }
    let Some(r) = app.canvas.ink_ui.replay.as_mut() else { return };
    match a {
        "rewind" => r.t = 0.0,
        "play" if r.playing => r.playing = false,
        "play" => {
            // Play again from the start once it has finished.
            if r.t >= r.end() {
                r.t = 0.0;
            }
            r.playing = true;
        }
        _ => {
            // Forward: finish the stroke being drawn, or the next one between strokes.
            let t = r.t;
            r.t = r.strokes.iter().map(|s| s.start + s.dur).find(|e| *e > t + 1e-3).unwrap_or(r.end());
        }
    }
    ui.ctx().request_repaint();
}

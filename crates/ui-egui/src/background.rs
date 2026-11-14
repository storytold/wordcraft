//! Picture Format › Remove Background (`ui.removeBackground`, #3344): the Background Removal mode
//! on the canvas, and Color › Set Transparent Color's pick (`ui.setTransparentColor`).
//!
//! Background Removal shows the selected picture whole, with what will be removed under a tint,
//! as the engine finds it (`cutout::foreground` on a small copy, again after each mark). Its tab
//! holds Mark Areas to Keep and Mark Areas to Remove (pens: drag on the picture to mark), Discard
//! All Changes and Keep Changes. Keep Changes, Enter, Esc or a press outside the picture run one
//! `picture.removeBackground` with the marks, so one undo step; Discard All Changes leaves the
//! picture as it was (and takes away a removal it had). Coming back to a picture shows its marks.
//! Only unturned pictures, as for crop mode.
//!
//! Set Transparent Color waits for a click on the picture and runs `picture.transparentColor`
//! with the point; Esc or a click elsewhere cancels it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, Painter, Pos2, Rect, Ui, pos2};
use image::RgbaImage;
use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::para::InlineObject;
use wordcraft_engine::cmd::objects::cutout::{self, DEFAULT_PEN, MAX_MARK_POINTS, MAX_MARKS, Mark};
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::frame;
use crate::theme::Tokens;

/// The contextual tab the mode shows.
pub const TAB: &str = "Background Removal";
/// The longest side of the copy the preview is found on (pixels).
const PREVIEW_MAX_PX: u32 = 480;
/// Pointer travel between two points of a mark (screen points).
const STEP: f32 = 2.0;

/// Preview textures outlive removal mode in egui memory, so separate sessions must not reuse
/// a generation even when their picture and position are the same.
static PREVIEW_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_generation() -> u64 {
    PREVIEW_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// Background Removal mode, on one picture.
pub struct Removal {
    pos: Pos,
    /// The picture's media when the mode started: the mode ends if it changes (undo, an agent).
    media: String,
    /// The bitmap the removal runs on (under the picture's other adjustments).
    base: String,
    marks: Vec<Mark>,
    /// Whether the picture had a removal when the mode started.
    had: bool,
    /// The pen: Mark Areas to Keep (true), to Remove (false), or none.
    pen: Option<bool>,
    /// A mark being drawn: its points on the picture and where the pointer last added one.
    stroke: Option<(Vec<[f32; 2]>, Pos2)>,
    /// Bumped when the marks change, so the preview is found again.
    generation: u64,
    /// The tab to go back to.
    prev_tab: String,
}

/// The selected picture, if unturned (it may be flipped).
struct Shown {
    pos: Pos,
    media: String,
    crop: [f32; 4],
    flip: (bool, bool),
}

fn shown(app: &WordApp) -> Option<Shown> {
    match crate::objects::selected(app)? {
        (pos, InlineObject::Image { media, crop, float, .. }) if wordcraft_geom::normalize_degrees(float.rot) == 0.0 => {
            Some(Shown { pos, media: media.clone(), crop: *crop, flip: (float.flip_h, float.flip_v) })
        }
        _ => None,
    }
}

/// The picture in Background Removal mode: the mode is on and that picture, unchanged, is still
/// the selection.
pub fn active(app: &WordApp) -> Option<Pos> {
    let r = app.canvas.bg_removal.as_ref()?;
    shown(app).filter(|s| s.pos == r.pos && s.media == r.media).map(|s| s.pos)
}

/// The pen in use: Mark Areas to Keep (true) or to Remove (false).
pub fn pen(app: &WordApp) -> Option<bool> {
    active(app).and(app.canvas.bg_removal.as_ref()).and_then(|r| r.pen)
}

/// Whether Remove Background (or Set Transparent Color) can start: an unturned picture is selected.
pub fn can_start(app: &WordApp) -> bool {
    shown(app).is_some()
}

fn turned_message(app: &mut WordApp) -> String {
    let msg = tl!("This works on unturned pictures: set Rotation to 0° first").to_string();
    app.status(msg.clone());
    msg
}

/// `ui.removeBackground`: Background Removal mode on the selected picture.
pub fn start(app: &mut WordApp) -> Result<Value, String> {
    if active(app).is_some() {
        return Ok(json!({"backgroundRemoval": true}));
    }
    let Some((_, InlineObject::Image { .. })) = crate::objects::selected(app) else {
        return Err(tl!("select a picture first").to_string());
    };
    let Some(s) = shown(app) else { return Err(turned_message(app)) };
    let look = wordcraft_engine::cmd::objects::look::current(&app.session).ok_or_else(|| tl!("select a picture first").to_string())?;
    let (had, marks) = match &look.cutout {
        Some(c) => (true, c.marks.clone()),
        None => (false, Vec::new()),
    };
    let prev_tab = if app.ui.tab == TAB { "Picture Format".to_string() } else { app.ui.tab.clone() };
    app.canvas.crop = None;
    app.canvas.crop_drag = None;
    app.canvas.pick_color = None;
    app.canvas.bg_removal =
        Some(Removal { pos: s.pos, media: s.media, base: look.base, marks, had, pen: None, stroke: None, generation: next_generation(), prev_tab });
    app.ui.tab = TAB.into();
    Ok(json!({"backgroundRemoval": true}))
}

/// The Background Removal tab's buttons: `keep` and `remove` pick (or put down) a pen, `discard`
/// leaves the picture as it was, `done` keeps the changes.
pub fn action(app: &mut WordApp, what: &str) -> Result<Value, String> {
    if active(app).is_none() {
        return Err(tl!("Remove Background first").to_string());
    }
    match what {
        "keep" | "remove" => {
            let keep = what == "keep";
            if let Some(r) = app.canvas.bg_removal.as_mut() {
                r.pen = if r.pen == Some(keep) { None } else { Some(keep) };
                return Ok(json!({"pen": r.pen}));
            }
            Ok(Value::Null)
        }
        "discard" => discard(app),
        _ => keep_changes(app),
    }
}

/// Keep Changes: the removal with the marks, one `picture.removeBackground`.
fn keep_changes(app: &mut WordApp) -> Result<Value, String> {
    let still = active(app).is_some();
    let Some(r) = app.canvas.bg_removal.take() else { return Ok(Value::Null) };
    restore_tab(app, &r);
    if !still {
        return Ok(Value::Null);
    }
    let marks = serde_json::to_value(&r.marks).unwrap_or(Value::Null);
    app.run("picture.removeBackground", json!({"marks": marks}))
}

/// Discard All Changes: the picture as it was before any removal.
fn discard(app: &mut WordApp) -> Result<Value, String> {
    let still = active(app).is_some();
    let Some(r) = app.canvas.bg_removal.take() else { return Ok(Value::Null) };
    restore_tab(app, &r);
    if still && r.had {
        return app.run("picture.removeBackground", json!({"discard": true}));
    }
    Ok(json!({"discarded": true}))
}

fn restore_tab(app: &mut WordApp, r: &Removal) {
    if app.ui.tab == TAB {
        app.ui.tab = r.prev_tab.clone();
    }
}

/// Once a frame: a mode whose picture is gone (another selection, undo) ends, changing nothing,
/// and so does a Set Transparent Color pick.
pub fn sync(app: &mut WordApp) {
    if app.canvas.bg_removal.is_some()
        && active(app).is_none()
        && let Some(r) = app.canvas.bg_removal.take()
    {
        restore_tab(app, &r);
    }
    if app.canvas.pick_color.is_some() && shown(app).map(|s| s.pos) != app.canvas.pick_color {
        app.canvas.pick_color = None;
    }
}

/// `ui.setTransparentColor`: the next click on the picture picks the colour to clear.
pub fn start_pick(app: &mut WordApp) -> Result<Value, String> {
    if active(app).is_some() {
        let msg = tl!("Keep or discard Background Removal changes before setting a transparent color").to_string();
        app.status(msg.clone());
        return Err(msg);
    }
    let Some((_, InlineObject::Image { .. })) = crate::objects::selected(app) else {
        return Err(tl!("select a picture first").to_string());
    };
    let Some(s) = shown(app) else { return Err(turned_message(app)) };
    app.canvas.pick_color = Some(s.pos);
    app.status(tl!("Click the color in the picture to make transparent (Esc cancels)"));
    Ok(json!({"pending": "transparentColor"}))
}

/// Esc and Enter: Keep Changes (Esc while marking drops that mark); Esc cancels a pick. Returns
/// true if the key was taken.
pub fn key(app: &mut WordApp, key: egui::Key) -> bool {
    if app.canvas.pick_color.is_some() && key == egui::Key::Escape {
        app.canvas.pick_color = None;
        return true;
    }
    if active(app).is_none() {
        return false;
    }
    if key == egui::Key::Escape
        && let Some(r) = app.canvas.bg_removal.as_mut()
        && r.stroke.take().is_some()
    {
        return true;
    }
    if matches!(key, egui::Key::Escape | egui::Key::Enter) {
        let _ = keep_changes(app);
        return true;
    }
    false
}

/// The picture in the mode (or being picked from), laid out: its page and frame on screen.
fn on_screen(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, pos: &Pos) -> Option<Rect> {
    let o = layout.object(pos, app.session.page_hint)?;
    pages.get(o.page).map(|p| frame::to_screen(*p, scale, o.rect))
}

/// Where screen point `at` is on the uncropped picture (0 to 1 across and down), for a picture
/// shown in `f`, cropped `crop` and flipped `flip`.
pub fn to_source(f: Rect, crop: [f32; 4], flip: (bool, bool), at: Pos2) -> [f32; 2] {
    let mut u = (at.x - f.min.x) / f.width().max(0.01);
    let mut v = (at.y - f.min.y) / f.height().max(0.01);
    if flip.0 {
        u = 1.0 - u;
    }
    if flip.1 {
        v = 1.0 - v;
    }
    [crop[0] + u * (1.0 - crop[0] - crop[2]).max(0.05), crop[1] + v * (1.0 - crop[1] - crop[3]).max(0.05)]
}

/// [`to_source`] the other way.
pub fn from_source(f: Rect, crop: [f32; 4], flip: (bool, bool), p: [f32; 2]) -> Pos2 {
    let mut u = (p[0] - crop[0]) / (1.0 - crop[0] - crop[2]).max(0.05);
    let mut v = (p[1] - crop[1]) / (1.0 - crop[1] - crop[3]).max(0.05);
    if flip.0 {
        u = 1.0 - u;
    }
    if flip.1 {
        v = 1.0 - v;
    }
    pos2(f.min.x + u * f.width(), f.min.y + v * f.height())
}

/// Presses and drags in the mode or a pick. `Some(true)`: taken; `None`: not about them (a press
/// outside the picture has just kept the changes or cancelled the pick, and goes on as usual).
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, pages: &[Rect], layout: &DocLayout, scale: f32, at: Pos2) -> Option<bool> {
    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer();
    if let Some(pos) = app.canvas.pick_color.clone() {
        if !pressed {
            return None;
        }
        app.canvas.pick_color = None;
        let s = shown(app)?;
        let f = on_screen(app, layout, pages, scale, &pos)?;
        if !f.contains(at) {
            return None;
        }
        let [x, y] = to_source(f, s.crop, s.flip, at);
        let _ = app.run("picture.transparentColor", json!({"x": x, "y": y}));
        return Some(true);
    }
    let pos = active(app)?;
    let s = shown(app)?;
    let f = on_screen(app, layout, pages, scale, &pos);
    let down = ui.input(|i| i.pointer.primary_down());
    let r = app.canvas.bg_removal.as_mut()?;
    if let Some((points, last)) = r.stroke.as_mut() {
        if down {
            let room = MAX_MARK_POINTS.saturating_sub(r.marks.iter().map(|m| m.points.len()).sum::<usize>());
            if let Some(f) = f
                && at.distance(*last) >= STEP
                && points.len() < room
            {
                points.push(to_source(f, s.crop, s.flip, at));
                *last = at;
            }
        } else if let (Some((points, _)), Some(keep)) = (r.stroke.take(), r.pen) {
            r.marks.push(Mark { keep, points, width: DEFAULT_PEN });
            r.generation = next_generation();
        }
        return Some(true);
    }
    if !pressed {
        return None;
    }
    let f = f?;
    if f.contains(at) {
        if r.pen.is_some() && r.marks.len() < MAX_MARKS && r.marks.iter().map(|m| m.points.len()).sum::<usize>() < MAX_MARK_POINTS {
            r.stroke = Some((vec![to_source(f, s.crop, s.flip, at)], at));
        }
        return Some(true);
    }
    // Outside the picture: Keep Changes, as in Word, and the press goes on.
    let _ = keep_changes(app);
    None
}

/// A pen over the picture in the mode, and a picker over it while picking.
pub fn cursor(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<egui::CursorIcon> {
    if let Some(pos) = &app.canvas.pick_color {
        return on_screen(app, layout, pages, scale, pos).filter(|f| f.contains(at)).map(|_| egui::CursorIcon::Crosshair);
    }
    let pos = active(app)?;
    let f = on_screen(app, layout, pages, scale, &pos)?;
    let marking = app.canvas.bg_removal.as_ref().is_some_and(|r| r.pen.is_some());
    (f.contains(at) || app.canvas.bg_removal.as_ref().is_some_and(|r| r.stroke.is_some())).then_some(if marking {
        egui::CursorIcon::Crosshair
    } else {
        egui::CursorIcon::Default
    })
}

/// The base bitmap shrunk for the preview, kept in egui's memory per media item.
fn thumb(app: &WordApp, ctx: &egui::Context, base: &str) -> Option<Arc<RgbaImage>> {
    let bytes = app.session.doc.media.get(base)?;
    type Cached = (Arc<Vec<u8>>, Option<Arc<RgbaImage>>);
    let id = egui::Id::unique("bg_removal_thumb");
    if let Some((_, t)) = ctx.data(|d| d.get_temp::<Cached>(id)).filter(|(old, _)| Arc::ptr_eq(old, bytes)) {
        return t;
    }
    let img = wordcraft_geom::picture::decode(bytes, wordcraft_geom::MAX_DECODED_BYTES).ok().map(|img| {
        let img = if img.width().max(img.height()) > PREVIEW_MAX_PX { img.thumbnail(PREVIEW_MAX_PX, PREVIEW_MAX_PX) } else { img };
        Arc::new(img.into_rgba8())
    });
    ctx.data_mut(|d| d.insert_temp(id, (bytes.clone(), img.clone())));
    img
}

/// The picture whole, what will be removed under `tint`: for `thumb` with `marks`.
pub fn preview_image(thumb: &RgbaImage, marks: &[Mark], tint: Color32) -> RgbaImage {
    let mask = cutout::foreground(thumb, marks);
    let mut out = thumb.clone();
    let t = [f32::from(tint.r()), f32::from(tint.g()), f32::from(tint.b())];
    for (p, m) in out.pixels_mut().zip(mask.pixels()) {
        let gone = 1.0 - f32::from(m.0[0]) / 255.0;
        if gone > 0.0 {
            let k = 0.8 * gone;
            let a = f32::from(p.0[3]) / 255.0;
            for (c, tv) in p.0.iter_mut().zip(t) {
                // Over white where the picture is see-through, so the tint shows the same.
                let under = f32::from(*c) * a + 255.0 * (1.0 - a);
                *c = (under * (1.0 - k) + tv * k).round() as u8;
            }
            p.0[3] = 255;
        }
    }
    out
}

/// The preview texture for the mode's marks, found again when they change (one kept at a time).
fn preview(app: &WordApp, ctx: &egui::Context, tint: Color32) -> Option<egui::TextureHandle> {
    let r = app.canvas.bg_removal.as_ref()?;
    let source = thumb(app, ctx, &r.base)?;
    type Cached = (u64, Color32, Arc<RgbaImage>, egui::TextureHandle);
    let id = egui::Id::unique("bg_removal_preview");
    if let Some((generation, color, old, tex)) = ctx.data(|d| d.get_temp::<Cached>(id))
        && generation == r.generation
        && color == tint
        && Arc::ptr_eq(&old, &source)
    {
        return Some(tex);
    }
    let img = preview_image(&source, &r.marks, tint);
    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    let tex = ctx.load_texture("bg_removal_preview", ci, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, (r.generation, tint, source, tex.clone())));
    Some(tex)
}

/// Draws the mode: the picture whole with the tint over what goes, the marks, and its frame.
/// Returns whether it drew (then the usual frame isn't drawn).
pub fn paint(app: &WordApp, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) -> bool {
    let Some(pos) = active(app) else { return false };
    let (Some(s), Some(f), Some(r)) = (shown(app), on_screen(app, layout, pages, scale, &pos), app.canvas.bg_removal.as_ref()) else { return false };
    painter.rect_filled(f, 0.0, Color32::WHITE);
    if let Some(tex) = preview(app, painter.ctx(), t.removed_tint) {
        // The part the crop shows, mirrored as the picture is.
        let [l, tp, rt, b] = s.crop;
        let (u0, u1) = if s.flip.0 { (1.0 - rt, l) } else { (l, 1.0 - rt) };
        let (v0, v1) = if s.flip.1 { (1.0 - b, tp) } else { (tp, 1.0 - b) };
        painter.image(tex.id(), f, Rect::from_min_max(pos2(u0, v0), pos2(u1, v1)), Color32::WHITE);
    }
    // The pen's width on screen: a share of the uncropped picture's longer side.
    let full = f.width() / (1.0 - s.crop[0] - s.crop[2]).max(0.05);
    let full = full.max(f.height() / (1.0 - s.crop[1] - s.crop[3]).max(0.05));
    let marks = r.marks.iter().map(|m| (m.keep, &m.points[..], m.width));
    let drawing = r.stroke.as_ref().zip(r.pen).map(|((p, _), keep)| (keep, &p[..], DEFAULT_PEN));
    let clip = painter.with_clip_rect(f);
    for (keep, points, width) in marks.chain(drawing) {
        let c = if keep { t.mark_keep } else { t.mark_remove };
        let pts: Vec<Pos2> = points.iter().map(|p| from_source(f, s.crop, s.flip, *p)).collect();
        let w = (width * full).max(2.0);
        if let [only] = pts[..] {
            clip.circle_filled(only, w / 2.0, c);
        } else {
            clip.add(egui::Shape::line(pts, egui::Stroke::new(w, c)));
        }
    }
    painter.rect_stroke(f, 0.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
    true
}

#[cfg(test)]
mod tests;

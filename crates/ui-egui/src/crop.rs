//! Crop mode on the canvas (Picture Format › Size › Crop, `ui.crop`): black crop handles replace
//! the selected picture's resize handles; dragging one trims that side (a corner, two sides)
//! while what stays visible keeps its scale and its place on the page, and dragging back out
//! restores what was cropped away (never past the source's edge). The cropped-away part shows
//! around the picture under a dark dim. Esc, Enter, a click outside the picture or Crop again
//! leave crop mode and keep the crop. A drop is one `picture.crop` with the crop and the new size,
//! so one undo step. Only unturned pictures: a turned one stays out of crop mode (status message).
//!
//! While dragging nothing is laid out: the preview is the picture's own source pixels, the part
//! that would stay visible bright and the rest dimmed.

use egui::{Color32, Painter, Pos2, Rect, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::para::InlineObject;
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::frame::{self, Handle};
use crate::theme::Tokens;

/// The most of the source one side can crop away (as `picture.crop` and the renderer clamp).
const MAX_SIDE: f32 = 0.95;
/// Pointer travel that turns a press on a handle into a drag (screen points).
const DRAG_START: f32 = 3.0;
/// Crop handles: arm length of a corner's L, length of an edge bar, thickness (screen points).
const ARM: f32 = 11.0;
const BAR: f32 = 16.0;
const THICK: f32 = 3.5;
/// The longest side of the source picture decoded for crop mode (pixels).
const SOURCE_MAX_PX: u32 = 1024;

/// Where a crop handle drag leaves a picture: its crop (left, top, right, bottom fractions of the
/// source), its new display size (points), and how far its frame's top-left corner moves (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cropped {
    pub crop: [f32; 4],
    pub w: f32,
    pub h: f32,
    pub dx: f32,
    pub dy: f32,
}

/// The display sizes `picture.crop` accepts (points).
const MIN_SIZE: f32 = 1.0;
const MAX_SIZE: f32 = 4000.0;

/// One axis of [`cropped`]: a `len` long side cropped `lo` / `hi`, its handle end `grab`
/// (-1 low, 1 high, 0 none) dragged by `d` points. Returns the new crops, length and how far the
/// low edge moved. The length stays within [`MIN_SIZE`]–[`MAX_SIZE`] at the same scale (the crop
/// stops there), unless it was already outside it.
fn axis(len: f32, lo: f32, hi: f32, grab: i8, d: f32) -> (f32, f32, f32, f32) {
    // The source's full length at this scale; the visible part is 1-lo-hi of it.
    let full = len / (1.0 - lo - hi).max(0.05);
    // How far the grabbed side's crop may go: at most MAX_SIDE, and the length within its range.
    let range = |cur: f32, other: f32| {
        let most = (1.0 - other - MIN_SIZE / full).min(MAX_SIDE - other).max(cur);
        let least = (1.0 - other - MAX_SIZE / full).max(0.0).min(cur);
        (least, most)
    };
    let (mut lo2, mut hi2) = (lo, hi);
    match grab {
        -1 => {
            let (least, most) = range(lo, hi);
            lo2 = (lo + d / full).max(least).min(most);
        }
        1 => {
            let (least, most) = range(hi, lo);
            hi2 = (hi - d / full).max(least).min(most);
        }
        _ => {}
    }
    (lo2, hi2, full * (1.0 - lo2 - hi2), (lo2 - lo) * full)
}

/// Drag crop handle `handle` of a `w`×`h` point picture cropped `crop` by (`dx`, `dy`) points: the
/// grabbed sides are trimmed (inward) or restored (outward) at the picture's current scale.
/// Hostile numbers (non-finite, non-positive sizes) leave it as it was; sizes are taken within
/// 1–4000 pt, as `picture.crop` takes them.
pub fn cropped(w: f32, h: f32, crop: [f32; 4], handle: Handle, dx: f32, dy: f32) -> Cropped {
    cropped_on_screen(w, h, crop, false, false, handle, dx, dy)
}

/// [`cropped`] for a picture flipped left to right (`flip_h`) and/or top to bottom (`flip_v`):
/// `handle` and the drag are on screen, the crop is the source's. Flipped, the side under a
/// handle is the source's opposite side, so that's the one that moves.
#[allow(clippy::too_many_arguments)]
pub fn cropped_on_screen(w: f32, h: f32, crop: [f32; 4], flip_h: bool, flip_v: bool, handle: Handle, dx: f32, dy: f32) -> Cropped {
    let fin = |x: f32| if x.is_finite() { x } else { 0.0 };
    let crop = crop.map(|c| fin(c).clamp(0.0, MAX_SIDE));
    let (w, h) = (fin(w), fin(h));
    let (dx, dy) = (fin(dx), fin(dy));
    if w <= 0.0 || h <= 0.0 || (dx == 0.0 && dy == 0.0) {
        return Cropped { crop, w, h, dx: 0.0, dy: 0.0 };
    }
    let (w, h) = (w.min(MAX_SIZE), h.min(MAX_SIZE));
    // On screen order: (left, right) and (top, bottom) as they show.
    let (sl, sr) = if flip_h { (crop[2], crop[0]) } else { (crop[0], crop[2]) };
    let (st, sb) = if flip_v { (crop[3], crop[1]) } else { (crop[1], crop[3]) };
    let (l, r, w2, mx) = axis(w, sl, sr, handle.x, dx);
    let (t, b, h2, my) = axis(h, st, sb, handle.y, dy);
    let (l, r) = if flip_h { (r, l) } else { (l, r) };
    let (t, b) = if flip_v { (b, t) } else { (t, b) };
    Cropped { crop: [l, t, r, b], w: w2, h: h2, dx: mx, dy: my }
}

/// The crop as it shows on screen: (left, top, right, bottom) of the displayed picture.
fn on_screen(crop: [f32; 4], flip_h: bool, flip_v: bool) -> [f32; 4] {
    let [l, t, r, b] = crop;
    let (l, r) = if flip_h { (r, l) } else { (l, r) };
    let (t, b) = if flip_v { (b, t) } else { (t, b) };
    [l, t, r, b]
}

/// A crop handle being dragged.
pub struct CropDrag {
    /// The picture being cropped: the drop applies only while it's still the one in crop mode.
    pos: Pos,
    handle: Handle,
    flip_h: bool,
    flip_v: bool,
    /// The picture's frame where the drag started (page points) and its page.
    page: usize,
    start: wordcraft_geom::Rect,
    /// Its display size and crop then.
    w: f32,
    h: f32,
    crop: [f32; 4],
    press: Pos2,
    moved: bool,
    /// Where the drag puts it now.
    now: Cropped,
}

impl CropDrag {
    fn follow(&mut self, at: Pos2, scale: f32) {
        if !self.moved && at.distance(self.press) < DRAG_START {
            return;
        }
        self.moved = true;
        let d = (at - self.press) / scale.max(0.01);
        self.now = cropped_on_screen(self.w, self.h, self.crop, self.flip_h, self.flip_v, self.handle, d.x, d.y);
    }
}

/// The selected picture, if it's unturned (it may be flipped).
struct Picture {
    pos: Pos,
    w: f32,
    h: f32,
    crop: [f32; 4],
    media: String,
    flip_h: bool,
    flip_v: bool,
}

fn picture(app: &WordApp) -> Option<Picture> {
    match crate::objects::selected(app)? {
        (pos, InlineObject::Image { w, h, crop, media, float, .. }) if wordcraft_geom::normalize_degrees(float.rot) == 0.0 => {
            Some(Picture { pos, w: *w, h: *h, crop: *crop, media: media.clone(), flip_h: float.flip_h, flip_v: float.flip_v })
        }
        _ => None,
    }
}

/// The picture in crop mode: crop mode is on and that picture is still the selection.
pub fn active(app: &WordApp) -> Option<Pos> {
    let on = app.canvas.crop.as_ref()?;
    picture(app).map(|p| p.pos).filter(|p| p == on)
}

/// Whether Crop can be clicked: a picture is selected (or crop mode is on, to leave it).
pub fn can_toggle(app: &WordApp) -> bool {
    active(app).is_some() || matches!(crate::objects::selected(app), Some((_, InlineObject::Image { .. })))
}

/// `ui.crop`: crop mode on for the selected picture, or off (keeping the crop).
pub fn toggle(app: &mut WordApp) -> Result<Value, String> {
    if active(app).is_some() {
        leave(app);
        return Ok(json!({"crop": false}));
    }
    let Some((_, InlineObject::Image { .. })) = crate::objects::selected(app) else {
        return Err(tl!("select a picture first").to_string());
    };
    let Some(Picture { pos, .. }) = picture(app) else {
        let msg = tl!("Crop on the page works on unturned pictures: set Rotation to 0° first, or use the crop boxes").to_string();
        app.status(msg.clone());
        return Err(msg);
    };
    app.canvas.crop = Some(pos);
    app.canvas.crop_drag = None;
    Ok(json!({"crop": true}))
}

fn leave(app: &mut WordApp) {
    app.canvas.crop = None;
    app.canvas.crop_drag = None;
}

/// Esc and Enter leave crop mode, keeping the crop. Returns true if the key was taken.
pub fn key(app: &mut WordApp, key: egui::Key) -> bool {
    if active(app).is_none() {
        leave(app);
        return false;
    }
    if matches!(key, egui::Key::Escape | egui::Key::Enter) {
        leave(app);
        return true;
    }
    false
}

fn screen(pages: &[Rect], scale: f32, page: usize, r: wordcraft_geom::Rect) -> Option<Rect> {
    pages.get(page).map(|p| frame::to_screen(*p, scale, r))
}

/// The picture in crop mode, laid out: its frame (page points) and page.
fn laid_out(app: &WordApp, layout: &DocLayout) -> Option<(usize, wordcraft_geom::Rect)> {
    let pos = active(app)?;
    layout.object(&pos, app.session.page_hint).map(|o| (o.page, o.rect))
}

/// Presses and drags in crop mode. `Some(true)`: taken; `Some(false)` / `None`: not about crop
/// mode, so the usual object handling goes on (a press outside the picture has just left it).
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, pages: &[Rect], layout: &DocLayout, scale: f32, at: Pos2) -> Option<bool> {
    if let Some(mut d) = app.canvas.crop_drag.take() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return Some(true); // the drag is dropped without a change; crop mode stays on
        }
        if active(app).as_ref() != Some(&d.pos) {
            // The picture changed under the drag (undo, another selection): drop it unapplied.
            return Some(true);
        }
        if ui.input(|i| i.pointer.primary_down()) {
            d.follow(at, scale);
            app.canvas.crop_drag = Some(d);
        } else if d.moved {
            let c = d.now;
            let _ = app.run(
                "picture.crop",
                json!({"left": c.crop[0], "top": c.crop[1], "right": c.crop[2], "bottom": c.crop[3], "width": c.w, "height": c.h}),
            );
        }
        return Some(true);
    }
    if active(app).is_none() {
        leave(app);
        return None;
    }
    if !(ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer()) {
        return None;
    }
    let (page, rect) = laid_out(app, layout)?;
    let Picture { pos, w, h, crop, flip_h, flip_v, .. } = picture(app)?;
    let f = screen(pages, scale, page, rect)?;
    if let Some(handle) = frame::handle_at(f, 0.0, at) {
        let now = Cropped { crop, w, h, dx: 0.0, dy: 0.0 };
        app.canvas.crop_drag = Some(CropDrag { pos, handle, flip_h, flip_v, page, start: rect, w, h, crop, press: at, moved: false, now });
        return Some(true);
    }
    if f.contains(at) {
        return Some(true); // inside the picture: crop mode stays on
    }
    // Outside: crop mode ends with the crop kept, and the press goes on as usual.
    leave(app);
    None
}

/// The cursor over a crop handle (or while dragging one).
pub fn cursor(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<egui::CursorIcon> {
    if let Some(d) = &app.canvas.crop_drag {
        return Some(d.handle.cursor());
    }
    let (page, rect) = laid_out(app, layout)?;
    let f = screen(pages, scale, page, rect)?;
    match frame::handle_at(f, 0.0, at) {
        Some(h) => Some(h.cursor()),
        None if f.contains(at) => Some(egui::CursorIcon::Default),
        None => None,
    }
}

/// The source picture as a texture (downscaled), kept in egui's memory per media item; None when
/// it can't be decoded (then the cropped-away area is just dimmed).
fn source(app: &WordApp, ctx: &egui::Context, media: &str) -> Option<egui::TextureHandle> {
    let bytes = app.session.doc.media.get(media)?;
    let id = egui::Id::unique(("crop_source", media, bytes.len()));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return t;
    }
    let tex = wordcraft_geom::picture::decode(bytes, wordcraft_geom::MAX_DECODED_BYTES).ok().map(|img| {
        let img = if img.width().max(img.height()) > SOURCE_MAX_PX { img.thumbnail(SOURCE_MAX_PX, SOURCE_MAX_PX) } else { img };
        let rgba = img.to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
        ctx.load_texture(format!("crop_source_{media}"), ci, egui::TextureOptions::LINEAR)
    });
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Draw crop mode: the cropped-away part dimmed around the picture, its frame and the crop
/// handles; while dragging, the picture as the drop would leave it. Returns whether crop mode
/// drew (then the usual frame isn't drawn).
pub fn paint(app: &WordApp, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) -> bool {
    let Some((page, rect)) = laid_out(app, layout) else { return false };
    let Some(Picture { crop, media, flip_h, flip_v, .. }) = picture(app) else { return false };
    let drag = app.canvas.crop_drag.as_ref().filter(|d| d.moved);
    let (page, frame_pt, c) = match drag {
        Some(d) => {
            let n = d.now;
            (d.page, wordcraft_geom::Rect::new(d.start.x + n.dx, d.start.y + n.dy, n.w, n.h), n.crop)
        }
        None => (page, rect, crop),
    };
    // The crop as it shows: a flipped picture shows its source's right on the left.
    let c = on_screen(c, flip_h, flip_v);
    // The whole source, at the picture's scale, around the visible part.
    let fw = frame_pt.w / (1.0 - c[0] - c[2]).max(0.05);
    let fh = frame_pt.h / (1.0 - c[1] - c[3]).max(0.05);
    let full_pt = wordcraft_geom::Rect::new(frame_pt.x - c[0] * fw, frame_pt.y - c[1] * fh, fw, fh);
    let (Some(f), Some(full)) = (screen(pages, scale, page, frame_pt), screen(pages, scale, page, full_pt)) else { return false };
    let tex = source(app, painter.ctx(), &media);
    // Where a screen rect is in the source (0–1), mirrored as the picture is flipped.
    let uv = |r: Rect| {
        let (w, h) = (full.width().max(0.01), full.height().max(0.01));
        let u = |x: f32| if flip_h { 1.0 - (x - full.min.x) / w } else { (x - full.min.x) / w };
        let v = |y: f32| if flip_v { 1.0 - (y - full.min.y) / h } else { (y - full.min.y) / h };
        Rect::from_min_max(pos2(u(r.min.x), v(r.min.y)), pos2(u(r.max.x), v(r.max.y)))
    };
    if let (Some(tex), Some(_)) = (&tex, drag) {
        // Dragging: the page still shows the old crop, so show the new visible part too.
        painter.rect_filled(f, 0.0, Color32::WHITE);
        painter.image(tex.id(), f, uv(f), Color32::WHITE);
    }
    // The cropped-away bands: top, bottom, left, right.
    let bands = [
        Rect::from_min_max(full.min, pos2(full.max.x, f.min.y)),
        Rect::from_min_max(pos2(full.min.x, f.max.y), full.max),
        Rect::from_min_max(pos2(full.min.x, f.min.y), pos2(f.min.x, f.max.y)),
        Rect::from_min_max(pos2(f.max.x, f.min.y), pos2(full.max.x, f.max.y)),
    ];
    for b in bands.into_iter().filter(|b| b.width() > 0.0 && b.height() > 0.0) {
        if let Some(tex) = &tex {
            painter.image(tex.id(), b, uv(b), Color32::WHITE);
        }
        painter.rect_filled(b, 0.0, t.crop_dim);
    }
    painter.rect_stroke(full, 0.0, egui::Stroke::new(1.0, t.crop_dim), egui::StrokeKind::Outside);
    painter.rect_stroke(f, 0.0, egui::Stroke::new(1.0, t.crop_handle), egui::StrokeKind::Inside);
    paint_handles(painter, t, f);
    true
}

/// The crop handles of frame `f`: an L at each corner, a bar mid-edge, drawn just inside the
/// frame in the crop handle colour with a light halo.
fn paint_handles(painter: &Painter, t: &Tokens, f: Rect) {
    for h in frame::handles(f) {
        let (sx, sy) = (f32::from(h.x), f32::from(h.y));
        let c = pos2(f.center().x + sx * f.width() / 2.0, f.center().y + sy * f.height() / 2.0);
        let bars: Vec<Rect> = match (h.x, h.y) {
            (0, _) => vec![Rect::from_two_pos(c - vec2(BAR / 2.0, 0.0), c + vec2(BAR / 2.0, -sy * THICK))],
            (_, 0) => vec![Rect::from_two_pos(c - vec2(0.0, BAR / 2.0), c + vec2(-sx * THICK, BAR / 2.0))],
            _ => vec![Rect::from_two_pos(c, c + vec2(-sx * ARM, -sy * THICK)), Rect::from_two_pos(c, c + vec2(-sx * THICK, -sy * ARM))],
        };
        for b in &bars {
            painter.rect_filled(b.expand(1.0), 0.0, t.handle);
        }
        for b in bars {
            painter.rect_filled(b, 0.0, t.crop_handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::para::{InlineObject, OBJ};
    use wordcraft_doc::{Document, Pos};

    use super::*;
    use crate::WordApp;

    const H: fn(i8, i8) -> Handle = |x, y| Handle { x, y };

    fn close(a: Cropped, b: Cropped) -> bool {
        let n = |x: f32, y: f32| (x - y).abs() < 1e-4;
        a.crop.iter().zip(b.crop).all(|(x, y)| n(*x, y)) && n(a.w, b.w) && n(a.h, b.h) && n(a.dx, b.dx) && n(a.dy, b.dy)
    }

    #[test]
    fn each_handle_trims_its_own_sides_and_keeps_the_scale() {
        let c = |crop, w, h, dx, dy| Cropped { crop, w, h, dx, dy };
        // Left in by 50 of a 200 pt wide picture: a quarter of the source goes.
        let r = cropped(200.0, 100.0, [0.0; 4], H(-1, 0), 50.0, 33.0);
        assert!(close(r, c([0.25, 0.0, 0.0, 0.0], 150.0, 100.0, 50.0, 0.0)), "{r:?}");
        // Right in by 40.
        let r = cropped(200.0, 100.0, [0.0; 4], H(1, 0), -40.0, 0.0);
        assert!(close(r, c([0.0, 0.0, 0.2, 0.0], 160.0, 100.0, 0.0, 0.0)), "{r:?}");
        // Bottom in by 10; the sideways part of the drag doesn't count.
        let r = cropped(200.0, 100.0, [0.0; 4], H(0, 1), 999.0, -10.0);
        assert!(close(r, c([0.0, 0.0, 0.0, 0.1], 200.0, 90.0, 0.0, 0.0)), "{r:?}");
        // Top-left corner: two sides, and the frame's corner follows.
        let r = cropped(200.0, 100.0, [0.0; 4], H(-1, -1), 20.0, 10.0);
        assert!(close(r, c([0.1, 0.1, 0.0, 0.0], 180.0, 90.0, 20.0, 10.0)), "{r:?}");
        // Already cropped: the full source is wider than the frame (160 of 200 shown).
        let r = cropped(160.0, 100.0, [0.2, 0.0, 0.0, 0.0], H(-1, 0), 20.0, 0.0);
        assert!(close(r, c([0.3, 0.0, 0.0, 0.0], 140.0, 100.0, 20.0, 0.0)), "{r:?}");
    }

    #[test]
    fn crops_are_clamped_and_never_go_past_the_source() {
        // At most 0.95 a side.
        let r = cropped(200.0, 100.0, [0.0; 4], H(-1, 0), 1000.0, 0.0);
        assert!((r.crop[0] - 0.95).abs() < 1e-4 && (r.w - 10.0).abs() < 1e-3 && (r.dx - 190.0).abs() < 1e-3, "{r:?}");
        // Out past the original edge: back to uncropped, no further.
        let r = cropped(160.0, 100.0, [0.0, 0.0, 0.2, 0.0], H(1, 0), 100.0, 0.0);
        assert!(close(r, Cropped { crop: [0.0; 4], w: 200.0, h: 100.0, dx: 0.0, dy: 0.0 }), "{r:?}");
        let r = cropped(200.0, 90.0, [0.0, 0.1, 0.0, 0.0], H(0, -1), 0.0, -500.0);
        assert!(close(r, Cropped { crop: [0.0; 4], w: 200.0, h: 100.0, dx: 0.0, dy: -10.0 }), "{r:?}");
    }

    /// #3135: a side can take almost the whole source (5% stays), so any part of the picture can
    /// be kept, not only its middle; the two sides of an axis keep their 5%.
    #[test]
    fn either_side_can_crop_almost_all_the_source_away() {
        // Drag the left handle 190 pt across a 200 pt picture: 10 pt (5%) stay.
        let r = cropped(200.0, 100.0, [0.0; 4], H(-1, 0), 190.0, 0.0);
        assert!((r.crop[0] - 0.95).abs() < 1e-4 && (r.w - 10.0).abs() < 1e-3 && (r.dx - 190.0).abs() < 1e-3, "{r:?}");
        // The right side already takes 0.6: the left stops at 0.35, leaving 5%.
        let r = cropped(80.0, 100.0, [0.0, 0.0, 0.6, 0.0], H(-1, 0), 1000.0, 0.0);
        assert!((r.crop[0] - 0.35).abs() < 1e-4 && (r.crop[2] - 0.6).abs() < 1e-4 && (r.w - 10.0).abs() < 1e-3, "{r:?}");
    }

    #[test]
    fn dragging_back_out_restores_what_was_cropped() {
        let a = cropped(200.0, 100.0, [0.0; 4], H(1, 1), -30.0, -20.0);
        let b = cropped(a.w, a.h, a.crop, H(1, 1), 30.0, 20.0);
        assert!(close(b, Cropped { crop: [0.0; 4], w: 200.0, h: 100.0, dx: 0.0, dy: 0.0 }), "{a:?} {b:?}");
    }

    #[test]
    fn hostile_input_leaves_the_picture_as_it_was() {
        let same = |w, h, crop| Cropped { crop, w, h, dx: 0.0, dy: 0.0 };
        // Junk drags: nothing moves.
        for (dx, dy) in [(f32::NAN, 0.0), (f32::INFINITY, f32::NEG_INFINITY), (f32::NAN, f32::NAN)] {
            let r = cropped(200.0, 100.0, [0.1, 0.0, 0.2, 0.0], H(-1, -1), dx, dy);
            assert_eq!(r, same(200.0, 100.0, [0.1, 0.0, 0.2, 0.0]), "{dx} {dy}");
        }
        // Junk sizes: as it was (a non-finite size reads as 0), nothing moves.
        assert_eq!(cropped(0.0, 100.0, [0.0; 4], H(-1, 0), 10.0, 0.0), same(0.0, 100.0, [0.0; 4]));
        assert_eq!(cropped(-5.0, 100.0, [0.0; 4], H(1, 1), 10.0, 10.0), same(-5.0, 100.0, [0.0; 4]));
        assert_eq!(cropped(200.0, f32::NAN, [0.0; 4], H(0, 1), 0.0, 10.0), same(200.0, 0.0, [0.0; 4]));
        // A junk crop reads as uncropped / at most 0.95 a side; a zero drag leaves that be.
        let r = cropped(200.0, 100.0, [f32::NAN, -1.0, 0.7, 0.0], H(-1, -1), 0.0, 0.0);
        assert_eq!(r, same(200.0, 100.0, [0.0, 0.0, 0.7, 0.0]));
    }

    /// Sizes stay within what `picture.crop` accepts (1–4000 pt) at the same scale, so the
    /// preview is finite and the drop doesn't drift.
    #[test]
    fn sizes_stay_within_1_to_4000_points() {
        // A 1.5 pt wide picture can lose only a third of itself before it'd be under 1 pt.
        let r = cropped(1.5, 100.0, [0.0; 4], H(-1, 0), 100.0, 0.0);
        assert!((r.w - 1.0).abs() < 1e-3 && (r.crop[0] - 1.0 / 3.0).abs() < 1e-3, "{r:?}");
        // Restoring a heavily cropped 4000 pt picture stops at 4000 pt.
        let r = cropped(4000.0, 100.0, [0.45, 0.0, 0.45, 0.0], H(1, 0), 1.0e7, 0.0);
        assert!(r.w <= 4000.0 + 1e-2 && r.w > 3999.0 && r.crop[2] > 0.0, "{r:?}");
        // Huge sizes from a file: clamped first, all finite.
        let r = cropped(f32::MAX, f32::MAX, [0.45; 4], H(1, 1), 1.0e30, 1.0e30);
        assert!([r.w, r.h, r.dx, r.dy].iter().chain(&r.crop).all(|x| x.is_finite()) && r.w <= 4000.01 && r.h <= 4000.01, "{r:?}");
    }

    /// Flipped, the handle on the screen's left is on the source's right: dragging it trims the
    /// side under it (as in Word), and the frame's left edge follows.
    #[test]
    fn flipped_pictures_trim_the_side_under_the_handle() {
        let c = |crop, w, h, dx, dy| Cropped { crop, w, h, dx, dy };
        let r = cropped_on_screen(200.0, 100.0, [0.0; 4], true, false, H(-1, 0), 50.0, 0.0);
        assert!(close(r, c([0.0, 0.0, 0.25, 0.0], 150.0, 100.0, 50.0, 0.0)), "flip H, left handle: {r:?}");
        let r = cropped_on_screen(200.0, 100.0, [0.0; 4], true, false, H(1, 0), -40.0, 0.0);
        assert!(close(r, c([0.2, 0.0, 0.0, 0.0], 160.0, 100.0, 0.0, 0.0)), "flip H, right handle: {r:?}");
        let r = cropped_on_screen(200.0, 100.0, [0.0; 4], false, true, H(0, -1), 0.0, 10.0);
        assert!(close(r, c([0.0, 0.0, 0.0, 0.1], 200.0, 90.0, 0.0, 10.0)), "flip V, top handle: {r:?}");
        // Already cropped on the source's left (the screen's right), flipped: the left handle
        // restores nothing there and trims the right of the source.
        let r = cropped_on_screen(160.0, 100.0, [0.2, 0.0, 0.0, 0.0], true, true, H(-1, -1), 20.0, 10.0);
        assert!(close(r, c([0.2, 0.0, 0.1, 0.1], 140.0, 90.0, 20.0, 10.0)), "flip both, corner: {r:?}");
    }

    /// An app whose first paragraph holds a picture, selected.
    fn app_with_picture(rot: f32) -> WordApp {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        let float = wordcraft_doc::para::Float { rot, ..Default::default() };
        let pic = InlineObject::Image { media: "m".into(), w: 200.0, h: 100.0, alt: String::new(), float, crop: [0.0; 4], ole: None };
        app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
        let end = Pos::body(0, OBJ.len_utf8());
        app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": end})).unwrap();
        app
    }

    #[test]
    fn ui_crop_toggles_crop_mode_and_esc_enter_leave_it_keeping_the_crop() {
        let mut app = app_with_picture(0.0);
        assert!(crate::widgets::enabled(&app, "ui.crop"));
        app.run("ui.crop", json!({})).unwrap();
        assert!(active(&app).is_some(), "Crop turns crop mode on");
        app.run("ui.crop", json!({})).unwrap();
        assert!(active(&app).is_none(), "Crop again turns it off");
        for key in [egui::Key::Escape, egui::Key::Enter] {
            app.run("ui.crop", json!({})).unwrap();
            app.run("picture.crop", json!({"left": 0.1})).unwrap();
            assert!(crate::objects::key(&mut app, key, Default::default()), "{key:?} is taken");
            assert!(active(&app).is_none(), "{key:?} leaves crop mode");
            let Some((_, InlineObject::Image { crop, .. })) = crate::objects::selected(&app) else { panic!("still selected") };
            assert_eq!(crop[0], 0.1, "{key:?} keeps the crop");
        }
        // Without crop mode, Escape isn't a crop key.
        assert!(!crate::objects::key(&mut app, egui::Key::Escape, Default::default()));
    }

    #[test]
    fn ui_crop_needs_an_unturned_selected_picture() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        assert!(!crate::widgets::enabled(&app, "ui.crop"));
        assert!(app.run("ui.crop", json!({})).is_err());
        assert!(active(&app).is_none());
        let mut app = app_with_picture(30.0);
        assert!(app.run("ui.crop", json!({})).is_err(), "a turned picture isn't cropped on the canvas yet");
        assert!(active(&app).is_none());
        // Selecting something else ends crop mode.
        let mut app = app_with_picture(0.0);
        app.run("ui.crop", json!({})).unwrap();
        app.run("caret.set", json!({"pos": Pos::body(0, 0)})).unwrap();
        assert!(active(&app).is_none());
    }

    use crate::table_resize::tests::{H, button, screen, steps};

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
        let mut v = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
        v
    }

    fn test_picture() -> InlineObject {
        InlineObject::Image { media: "m".into(), w: 200.0, h: 100.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None }
    }

    /// The whole window with `n` 200×100 pt pictures in the first paragraph, the first one
    /// selected and in crop mode.
    fn crop_window(n: usize) -> H {
        let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        a.session.doc.media.insert("m".into(), std::sync::Arc::new(png(40, 20)));
        for _ in 0..n {
            a.session.doc.insert_object(&Pos::body(0, 0), test_picture(), &Default::default()).unwrap();
        }
        a.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, OBJ.len_utf8())})).unwrap();
        a.run("ui.crop", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        steps(&mut h, 6);
        h
    }

    fn rect_of(h: &mut H, off: usize) -> wordcraft_geom::Rect {
        h.state_mut().session.layout().object(&Pos::body(0, off), 0).expect("laid out").rect
    }

    fn image_at(h: &H, off: usize) -> (f32, f32, [f32; 4]) {
        match h.state().session.doc.para_at(&Pos::body(0, off)).and_then(|p| p.object_at(off)) {
            Some(InlineObject::Image { w, h, crop, .. }) => (*w, *h, *crop),
            other => panic!("expected a picture, got {other:?}"),
        }
    }

    /// Press at `from` and move to `to` in a few steps, still holding the button.
    fn drag(h: &mut H, from: egui::Pos2, to: egui::Pos2) {
        h.event(egui::Event::PointerMoved(from));
        h.step();
        button(h, from, true);
        h.step();
        for i in 1..=4 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
            h.step();
        }
    }

    /// A real mouse drag of the bottom-right crop handle: one `picture.crop` on release, the
    /// scale kept, crop mode still on; one undo restores the picture.
    #[test]
    fn dragging_a_crop_handle_with_the_mouse_is_one_undo_step() {
        let mut h = crop_window(1);
        let r = rect_of(&mut h, 0);
        let depth = h.state().session.undo_depth();
        let from = screen(&mut h, r.right(), r.bottom());
        let to = from + egui::vec2(-40.0, -20.0);
        drag(&mut h, from, to);
        assert_eq!(image_at(&h, 0), (200.0, 100.0, [0.0; 4]), "nothing changes while dragging");
        button(&h, to, false);
        steps(&mut h, 3);
        let (w, ht, crop) = image_at(&h, 0);
        assert!(crop[2] > 0.05 && crop[3] > 0.05 && crop[0] == 0.0 && crop[1] == 0.0, "{crop:?}");
        assert!((w / (1.0 - crop[2]) - 200.0).abs() < 0.5 && (ht / (1.0 - crop[3]) - 100.0).abs() < 0.5, "scale kept: {w}×{ht} {crop:?}");
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");
        assert!(active(h.state()).is_some(), "crop mode stays on after a drop");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(image_at(&h, 0), (200.0, 100.0, [0.0; 4]));
    }

    /// A click outside the picture leaves crop mode, keeping the crop.
    #[test]
    fn a_click_outside_the_picture_leaves_crop_mode_and_keeps_the_crop() {
        let mut h = crop_window(1);
        h.state_mut().run("picture.crop", json!({"left": 0.1, "width": 180.0, "height": 100.0})).unwrap();
        steps(&mut h, 2);
        let r = rect_of(&mut h, 0);
        let out = screen(&mut h, r.right() + 150.0, r.bottom() + 150.0);
        h.event(egui::Event::PointerMoved(out));
        h.step();
        button(&h, out, true);
        h.step();
        button(&h, out, false);
        steps(&mut h, 3);
        assert!(active(h.state()).is_none(), "crop mode is off");
        assert_eq!(image_at(&h, 0).2, [0.1, 0.0, 0.0, 0.0], "the crop is kept");
    }

    /// When the selection moves to another picture mid-drag (an agent, Ctrl+Z…), the release
    /// changes nothing: neither the picture the drag started on nor the newly selected one.
    #[test]
    fn a_crop_drag_never_lands_on_another_picture() {
        let mut h = crop_window(2);
        let r = rect_of(&mut h, 0);
        let depth = h.state().session.undo_depth();
        let from = screen(&mut h, r.right(), r.bottom());
        let to = from + egui::vec2(-40.0, -20.0);
        drag(&mut h, from, to);
        let second = Pos::body(0, OBJ.len_utf8());
        h.state_mut().run("select.range", json!({"anchor": second, "focus": Pos::body(0, 2 * OBJ.len_utf8())})).unwrap();
        let depth = depth.max(h.state().session.undo_depth());
        button(&h, to, false);
        steps(&mut h, 3);
        assert_eq!(image_at(&h, 0), (200.0, 100.0, [0.0; 4]));
        assert_eq!(image_at(&h, OBJ.len_utf8()), (200.0, 100.0, [0.0; 4]));
        assert_eq!(h.state().session.undo_depth(), depth, "no edit");
        assert!(h.state().canvas.crop_drag.is_none());
    }
}

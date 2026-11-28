//! Crop mode on the canvas (Picture Format › Size › Crop, `ui.crop`): the selected picture is
//! edited through a `CropOverlay` scoped to the picture's source. Dragging a side trims it while
//! what stays visible keeps its scale and its place on the page, and dragging back out restores
//! what was cropped away (never past the source's edge). Esc, Enter, a click outside the picture
//! or Crop again leave crop mode and keep the crop. A drop is one `picture.crop` with the crop and
//! the new size, so one undo step. Only unturned pictures: a turned one stays out of crop mode
//! (status message).

use craft::behavior::geometry::CropBox;
use craft::behavior::kurbo::{Point, Rect as DocRect};
use craft::behavior::{CanvasView, EditEvent, Input, Metrics};
use craft::pages::ui::{PageBody, PlacedPage};
use craft::ui::canvas::{CropOverlay, CropScope};
use craft::ui::contract::{Access, Bind, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft::ui::primitives::TextureCache;
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, StyleCx};
use egui::{Rect, Response, TextureId, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::para::InlineObject;
use wordcraft_layout::DocLayout;

use crate::WordApp;

/// The most of the source one side can crop away (as `picture.crop` and the renderer clamp).
const MAX_SIDE: f32 = 0.95;

/// The longest side of the source texture, in pixels.
const SOURCE_MAX_PX: u32 = 1024;

/// Where a crop drag leaves a picture: its crop (left, top, right, bottom fractions of the
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

/// One axis of [`crop_of`]: the crops (`lo`, `hi` as fractions of a `full` long source) the new
/// edges ask for, clamped to [`MAX_SIDE`] a side and to a visible length within
/// [`MIN_SIZE`]–[`MAX_SIZE`] at the same scale (unless it was already outside). Where the two sides
/// together go too far, the side that moved more gives way.
fn axis(full: f32, old: (f32, f32), new: (f32, f32)) -> (f32, f32) {
    let (mut lo, mut hi) = (new.0.clamp(0.0, MAX_SIDE), new.1.clamp(0.0, MAX_SIDE));
    let lo_moved = (lo - old.0).abs() >= (hi - old.1).abs();
    let old_sum = old.0 + old.1;
    let most = MAX_SIDE.min(1.0 - MIN_SIZE / full).max(old_sum);
    let least = (1.0 - MAX_SIZE / full).clamp(0.0, most).min(old_sum);
    let total = lo + hi;
    let d = total.clamp(least, most) - total;
    if lo_moved {
        lo = (lo + d).clamp(0.0, MAX_SIDE);
    } else {
        hi = (hi + d).clamp(0.0, MAX_SIDE);
    }
    (lo, hi)
}

/// Where a picture lands when the crop frame the overlay reports is `frame`: `source` is the whole
/// source picture at the picture's scale and `crop0` the crop it has now (both in page points),
/// `flips` whether it is flipped left to right and top to bottom. The frame's sides are on screen;
/// flipped, the side under one is the source's opposite side, so that is the crop that changes.
/// Hostile numbers (non-finite, non-positive sizes) leave the picture as it was; sizes are taken
/// within 1–4000 pt, as `picture.crop` takes them.
pub fn crop_of(source: Rect, frame: Rect, crop0: [f32; 4], flips: (bool, bool)) -> Cropped {
    let fin = |x: f32| if x.is_finite() { x } else { 0.0 };
    let crop0 = crop0.map(|c| fin(c).clamp(0.0, MAX_SIDE));
    let shown = on_screen(crop0, flips.0, flips.1);
    let (fw, fh) = (source.width(), source.height());
    let old_w = fin(fw * (1.0 - shown[0] - shown[2]));
    let old_h = fin(fh * (1.0 - shown[1] - shown[3]));
    let same = Cropped { crop: crop0, w: old_w, h: old_h, dx: 0.0, dy: 0.0 };
    let finite = |r: Rect| r.min.is_finite() && r.max.is_finite();
    if !finite(source) || !finite(frame) || !(fw > 0.0 && fh > 0.0 && fw.is_finite() && fh.is_finite()) {
        return same;
    }
    let (l, r) = ((frame.min.x - source.min.x) / fw, (source.max.x - frame.max.x) / fw);
    let (t, b) = ((frame.min.y - source.min.y) / fh, (source.max.y - frame.max.y) / fh);
    if ![l, r, t, b].iter().all(|x| x.is_finite()) {
        return same;
    }
    let (l, r) = axis(fw, (shown[0], shown[2]), (l, r));
    let (t, b) = axis(fh, (shown[1], shown[3]), (t, b));
    let (w, h) = (fw * (1.0 - l - r), fh * (1.0 - t - b));
    let old_left = shown[0] * fw;
    let old_top = shown[1] * fh;
    let out = Cropped { crop: on_screen([l, t, r, b], flips.0, flips.1), w, h, dx: l * fw - old_left, dy: t * fh - old_top };
    if [out.w, out.h, out.dx, out.dy].iter().chain(&out.crop).all(|x| x.is_finite()) { out } else { same }
}

/// The crop as it shows on screen: (left, top, right, bottom) of the displayed picture. The map is
/// its own inverse, so it also turns a screen crop into the source's.
fn on_screen(crop: [f32; 4], flip_h: bool, flip_v: bool) -> [f32; 4] {
    let [l, t, r, b] = crop;
    let (l, r) = if flip_h { (r, l) } else { (l, r) };
    let (t, b) = if flip_v { (b, t) } else { (t, b) };
    [l, t, r, b]
}

/// The selected picture, if it's unturned (it may be flipped).
struct Picture {
    pos: Pos,
    media: String,
    crop: [f32; 4],
    flip_h: bool,
    flip_v: bool,
}

fn picture(app: &WordApp) -> Option<Picture> {
    match crate::objects::selected(app)? {
        (pos, InlineObject::Image { crop, float, media, .. }) if wordcraft_geom::normalize_degrees(float.rot) == 0.0 => {
            Some(Picture { pos, media: media.clone(), crop: *crop, flip_h: float.flip_h, flip_v: float.flip_v })
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
    Ok(json!({"crop": true}))
}

/// Crop mode off, keeping the crop. The object path calls it when a press is not on the cropped picture.
pub(crate) fn leave(app: &mut WordApp) {
    app.canvas.crop = None;
}

/// The picture in crop mode, laid out: its page and frame (page points).
fn laid_out(app: &WordApp, layout: &DocLayout) -> Option<(usize, Rect)> {
    let pos = active(app)?;
    layout.object(&pos, app.session.page_hint).map(|o| (o.page, Rect::from_min_size(pos2(o.rect.x, o.rect.y), vec2(o.rect.w, o.rect.h))))
}

/// A page-point rect in document units.
fn to_doc_rect(page: &PlacedPage, view: &CanvasView, r: Rect) -> DocRect {
    let a = page.to_doc(view, Point::new(f64::from(r.min.x), f64::from(r.min.y)));
    let b = page.to_doc(view, Point::new(f64::from(r.max.x), f64::from(r.max.y)));
    DocRect::new(a.x, a.y, b.x, b.y)
}

/// A document-unit rect in page points.
fn from_doc_rect(page: &PlacedPage, view: &CanvasView, r: DocRect) -> Rect {
    let a = page.from_doc(view, Point::new(r.x0, r.y0));
    let b = page.from_doc(view, Point::new(r.x1, r.y1));
    Rect::from_min_max(pos2(a.x as f32, a.y as f32), pos2(b.x as f32, b.y as f32))
}

/// The source picture as a texture (downscaled), kept in the texture cache per media item; `None` when the
/// media is missing. A source that cannot be decoded is a transparent texture, so the cropped-away area is
/// just shielded.
fn source_texture(app: &WordApp, ctx: &egui::Context, media: &str) -> Option<TextureId> {
    let bytes = app.session.doc.media.get(media)?;
    Some(TextureCache::texture(ctx, (media.to_owned(), bytes.len()), || {
        let Ok(img) = wordcraft_geom::picture::decode(bytes, wordcraft_geom::MAX_DECODED_BYTES) else {
            return egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT);
        };
        let img = if img.width().max(img.height()) > SOURCE_MAX_PX { img.thumbnail(SOURCE_MAX_PX, SOURCE_MAX_PX) } else { img };
        let rgba = img.to_rgba8();
        egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
    }))
}

/// The look of [`CropSource`]: it has none of its own, the picture is the content.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct CropSourceStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The whole source picture at the picture's scale, under the crop overlay: the overlay's shield dims what
/// the crop cuts away, and dragging a side back out shows what it restores. Display only.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
struct CropSource {
    #[common]
    common: Common,
    /// The source on screen.
    rect: Rect,
    texture: TextureId,
    /// The part of the texture shown: flipped when the picture is.
    uv: Rect,
}

impl CropSource {
    fn new(label: impl Into<Label>, rect: Rect, texture: TextureId, flips: (bool, bool)) -> Self {
        let (u0, u1) = if flips.0 { (1.0, 0.0) } else { (0.0, 1.0) };
        let (v0, v1) = if flips.1 { (1.0, 0.0) } else { (0.0, 1.0) };
        CropSource { common: Common::new(), rect, texture, uv: Rect::from_min_max(pos2(u0, v0), pos2(u1, v1)) }.label(label)
    }
}

impl Component for CropSource {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = CropSourceStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: CropSourceStyleKey) -> CropSourceStyle {
        CropSourceStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        ui.draw().texture(self.rect, self.texture, self.uv);
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The crop overlay of the picture in crop mode: its frame over the picture, bounded by its
/// source. A drag is one `picture.crop`; Enter or Esc leaves crop mode keeping the crop. A press
/// outside the picture claims no overlay part, so it goes to the object path, which leaves crop
/// mode (`leave`).
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody<'_>) {
    if active(app).is_none() {
        leave(app);
        return;
    }
    let (Some((page, frame)), Some(pic)) = (laid_out(app, layout), picture(app)) else { return };
    let Some(placed) = body.pages.iter().find(|p| p.page == page) else { return };
    let c = on_screen(pic.crop, pic.flip_h, pic.flip_v);
    let full_w = frame.width() / (1.0 - c[0] - c[2]).max(0.05);
    let full_h = frame.height() / (1.0 - c[1] - c[3]).max(0.05);
    let source = Rect::from_min_size(pos2(frame.min.x - c[0] * full_w, frame.min.y - c[1] * full_h), vec2(full_w, full_h));
    if let Some(texture) = source_texture(app, ui.ctx(), &pic.media) {
        let at = |p: egui::Pos2| placed.to_screen(Point::new(f64::from(p.x), f64::from(p.y)));
        let on_screen = Rect::from_two_pos(at(source.min), at(source.max));
        CropSource::new(tl!("Picture source").to_string(), on_screen, texture, (pic.flip_h, pic.flip_v)).show_at(ui, body.rect);
    }
    let crop_box = CropBox::upright(to_doc_rect(placed, &body.view, frame));
    let scope = CropScope::object(to_doc_rect(placed, &body.view, source));
    // Given, not bound: the box is made from the document each pass and only the drop's Commit changes the document, so
    // the box stays at its pass-start value while a drag runs; a bound value that differs from the drag's last update
    // reads as the document moving under the gesture, which cancels it.
    let out = CropOverlay::new(tl!("Crop").to_string(), Bind::given(crop_box)).scope(scope).show_at(ui, body.rect);
    for event in out.edit.events {
        if let EditEvent::Commit { after, .. } = event {
            let c = crop_of(source, from_doc_rect(placed, &body.view, after.rect), pic.crop, (pic.flip_h, pic.flip_v));
            let _ = app.run(
                "picture.crop",
                json!({"left": c.crop[0], "top": c.crop[1], "right": c.crop[2], "bottom": c.crop[3], "width": c.w, "height": c.h}),
            );
        }
    }
    if out.apply.is_some() || out.discard {
        leave(app);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::para::{InlineObject, OBJ};
    use wordcraft_doc::{Document, Pos};

    use super::*;
    use crate::WordApp;
    use crate::table_resize::tests::{H, button, screen, steps};
    use crate::test_harness::AppFrames as _;

    fn close(a: Cropped, b: Cropped) -> bool {
        let n = |x: f32, y: f32| (x - y).abs() < 1e-4;
        a.crop.iter().zip(b.crop).all(|(x, y)| n(*x, y)) && n(a.w, b.w) && n(a.h, b.h) && n(a.dx, b.dx) && n(a.dy, b.dy)
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    /// The whole source: 200×100 pt at the page's origin.
    fn src() -> Rect {
        rect(0.0, 0.0, 200.0, 100.0)
    }

    #[test]
    fn each_handle_trims_its_own_sides_and_keeps_the_scale() {
        let c = |crop, w, h, dx, dy| Cropped { crop, w, h, dx, dy };
        let r = crop_of(src(), rect(50.0, 0.0, 200.0, 100.0), [0.0; 4], (false, false));
        assert!(close(r, c([0.25, 0.0, 0.0, 0.0], 150.0, 100.0, 50.0, 0.0)), "{r:?}");
        let r = crop_of(src(), rect(0.0, 0.0, 160.0, 100.0), [0.0; 4], (false, false));
        assert!(close(r, c([0.0, 0.0, 0.2, 0.0], 160.0, 100.0, 0.0, 0.0)), "{r:?}");
        let r = crop_of(src(), rect(0.0, 0.0, 200.0, 90.0), [0.0; 4], (false, false));
        assert!(close(r, c([0.0, 0.0, 0.0, 0.1], 200.0, 90.0, 0.0, 0.0)), "{r:?}");
        let r = crop_of(src(), rect(20.0, 10.0, 200.0, 100.0), [0.0; 4], (false, false));
        assert!(close(r, c([0.1, 0.1, 0.0, 0.0], 180.0, 90.0, 20.0, 10.0)), "{r:?}");
        // Already cropped 0.2 on the left (the frame starts at 40): the next trim adds to it.
        let r = crop_of(src(), rect(60.0, 0.0, 200.0, 100.0), [0.2, 0.0, 0.0, 0.0], (false, false));
        assert!(close(r, c([0.3, 0.0, 0.0, 0.0], 140.0, 100.0, 20.0, 0.0)), "{r:?}");
    }

    #[test]
    fn crops_are_clamped_and_never_go_past_the_source() {
        // At most 0.95 a side, whatever the frame asks.
        let r = crop_of(src(), rect(195.0, 0.0, 200.0, 100.0), [0.0; 4], (false, false));
        assert!((r.crop[0] - 0.95).abs() < 1e-4 && (r.w - 10.0).abs() < 1e-3 && (r.dx - 190.0).abs() < 1e-3, "{r:?}");
        // The right side already takes 0.6: the left stops at 0.35, leaving 5%.
        let r = crop_of(src(), rect(79.0, 0.0, 80.0, 100.0), [0.0, 0.0, 0.6, 0.0], (false, false));
        assert!((r.crop[0] - 0.35).abs() < 1e-4 && (r.crop[2] - 0.6).abs() < 1e-4 && (r.w - 10.0).abs() < 1e-3, "{r:?}");
        // Out past the source's edge: back to uncropped, no further.
        let r = crop_of(src(), rect(0.0, 0.0, 260.0, 100.0), [0.0, 0.0, 0.2, 0.0], (false, false));
        assert!(close(r, Cropped { crop: [0.0; 4], w: 200.0, h: 100.0, dx: 0.0, dy: 0.0 }), "{r:?}");
        // Dragging back out restores what was cropped.
        let a = crop_of(src(), rect(0.0, 0.0, 170.0, 80.0), [0.0; 4], (false, false));
        let b = crop_of(src(), rect(0.0, 0.0, 200.0, 100.0), a.crop, (false, false));
        assert!(close(b, Cropped { crop: [0.0; 4], w: 200.0, h: 100.0, dx: 0.0, dy: 0.0 }), "{a:?} {b:?}");
    }

    #[test]
    fn hostile_input_leaves_the_picture_as_it_was() {
        let same = |w, h, crop| Cropped { crop, w, h, dx: 0.0, dy: 0.0 };
        let crop = [0.1, 0.0, 0.2, 0.0];
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let r = crop_of(src(), rect(bad, 0.0, 100.0, bad), crop, (false, false));
            assert_eq!(r, same(140.0, 100.0, crop), "{bad}");
        }
        // Junk sources: nothing moves.
        let r = crop_of(rect(0.0, 0.0, 0.0, 100.0), rect(0.0, 0.0, 10.0, 10.0), [0.0; 4], (false, false));
        assert_eq!((r.dx, r.dy, r.crop), (0.0, 0.0, [0.0; 4]));
        let r = crop_of(rect(0.0, 0.0, f32::NAN, 100.0), src(), [0.0; 4], (false, false));
        assert_eq!((r.dx, r.dy, r.crop), (0.0, 0.0, [0.0; 4]));
        // A junk crop reads as uncropped / at most 0.95 a side; a frame where the cleaned crop has it (no drag) leaves
        // that be. (A frame over the whole source says the crop was dragged out, which restores it.)
        let r = crop_of(src(), rect(0.0, 0.0, 60.0, 100.0), [f32::NAN, -1.0, 0.7, 0.0], (false, false));
        assert_eq!(r.crop, [0.0, 0.0, 0.7, 0.0]);
    }

    /// Sizes stay within what `picture.crop` accepts (1–4000 pt) at the same scale, so the
    /// preview is finite and the drop doesn't drift.
    #[test]
    fn sizes_stay_within_1_to_4000_points() {
        // A 1.5 pt wide source can lose only a third of itself before it'd be under 1 pt.
        let r = crop_of(rect(0.0, 0.0, 1.5, 100.0), rect(1.4, 0.0, 1.5, 100.0), [0.0; 4], (false, false));
        assert!((r.w - 1.0).abs() < 1e-3 && (r.crop[0] - 1.0 / 3.0).abs() < 1e-3, "{r:?}");
        // Restoring a heavily cropped 4000 pt picture (a 40000 pt source) stops at 4000 pt.
        let big = rect(0.0, 0.0, 40000.0, 100.0);
        let r = crop_of(big, rect(18000.0, 0.0, 40000.0, 100.0), [0.45, 0.0, 0.45, 0.0], (false, false));
        assert!(r.w <= 4000.0 + 1e-2 && r.w > 3999.0 && r.crop[2] > 0.0, "{r:?}");
        // Huge sizes from a file: all finite.
        let r = crop_of(rect(0.0, 0.0, f32::MAX, f32::MAX), rect(1.0e30, 1.0e30, f32::MAX, f32::MAX), [0.45; 4], (false, false));
        assert!([r.w, r.h, r.dx, r.dy].iter().chain(&r.crop).all(|x| x.is_finite()), "{r:?}");
    }

    /// Flipped, the frame's left side on screen is the source's right: trimming it crops that
    /// side, and the frame's left edge follows.
    #[test]
    fn flipped_pictures_trim_the_side_under_the_handle() {
        let c = |crop, w, h, dx, dy| Cropped { crop, w, h, dx, dy };
        let r = crop_of(src(), rect(50.0, 0.0, 200.0, 100.0), [0.0; 4], (true, false));
        assert!(close(r, c([0.0, 0.0, 0.25, 0.0], 150.0, 100.0, 50.0, 0.0)), "flip H, left side: {r:?}");
        let r = crop_of(src(), rect(0.0, 0.0, 160.0, 100.0), [0.0; 4], (true, false));
        assert!(close(r, c([0.2, 0.0, 0.0, 0.0], 160.0, 100.0, 0.0, 0.0)), "flip H, right side: {r:?}");
        let r = crop_of(src(), rect(0.0, 10.0, 200.0, 100.0), [0.0; 4], (false, true));
        assert!(close(r, c([0.0, 0.0, 0.0, 0.1], 200.0, 90.0, 0.0, 10.0)), "flip V, top side: {r:?}");
    }

    /// An app whose first paragraph holds a picture, selected.
    fn app_with_picture(rot: f32) -> WordApp {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        let float = wordcraft_doc::para::Float { rot, ..Default::default() };
        let pic = InlineObject::Image {
            media: "m".into(),
            w: 200.0,
            h: 100.0,
            alt: String::new(),
            float,
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        };
        app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
        let end = Pos::body(0, OBJ.len_utf8());
        app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": end})).unwrap();
        app
    }

    #[test]
    fn ui_crop_toggles_crop_mode() {
        let mut app = app_with_picture(0.0);
        app.run("ui.crop", json!({})).unwrap();
        assert!(active(&app).is_some(), "Crop turns crop mode on");
        app.run("ui.crop", json!({})).unwrap();
        assert!(active(&app).is_none(), "Crop again turns it off");
    }

    /// Esc and Enter reach the overlay as keys of the canvas: crop mode ends and the crop stays.
    #[test]
    fn esc_and_enter_leave_crop_mode_keeping_the_crop() {
        let mut h = crop_window(1);
        for key in [egui::Key::Escape, egui::Key::Enter] {
            if active(h.state()).is_none() {
                h.state_mut().run("ui.crop", json!({})).unwrap();
            }
            h.state_mut().run("picture.crop", json!({"left": 0.1})).unwrap();
            steps(&mut h, 2);
            h.key_press(key);
            steps(&mut h, 3);
            assert!(active(h.state()).is_none(), "{key:?} leaves crop mode");
            assert_eq!(image_at(&h, 0).2[0], 0.1, "{key:?} keeps the crop");
        }
    }

    #[test]
    fn ui_crop_needs_an_unturned_selected_picture() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
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

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));

        wordcraft_geom::picture::encode_format(&image::DynamicImage::from(img.clone()), image::ImageFormat::Png).unwrap()
    }

    fn test_picture() -> InlineObject {
        InlineObject::Image {
            media: "m".into(),
            w: 200.0,
            h: 100.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        }
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
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
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
        h.app_frame();
        button(h, from, true);
        h.app_frame();
        for i in 1..=4 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
            h.app_frame();
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
        h.app_frame();
        button(&h, out, true);
        h.app_frame();
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
    }
}

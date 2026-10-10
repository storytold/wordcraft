//! A transform frame: the outline and handles around something on a page that is moved by
//! dragging it, resized by dragging a handle and turned by dragging the rotation handle above
//! it. It knows nothing about what it frames: callers give a page rect (points) and its angle and
//! get back where the drag puts it, so pictures, shapes, text boxes and anything else with a page
//! rect share it. A turned frame is the rect turned about its centre.

use egui::{CursorIcon, Painter, Pos2, Stroke, pos2, vec2};
use wordcraft_geom::Rect;

use crate::theme::Tokens;

/// Handle radius, the radius a press grabs a handle within, and the pointer travel that turns a
/// press into a drag (screen points).
const HANDLE_R: f32 = 4.5;
const GRAB_R: f32 = 8.0;
const DRAG_START: f32 = 3.0;
/// Frames smaller than this (screen points) show only corner handles on that axis.
const EDGE_HANDLES_FROM: f32 = 40.0;
/// How far the rotation handle sits above the frame's top edge (screen points), and the step
/// Shift snaps the angle to (degrees).
const ROTATE_GAP: f32 = 22.0;
const SNAP_DEG: f32 = 15.0;

/// Screen point `p` turned `deg` degrees clockwise about `c`.
fn turn(p: Pos2, c: Pos2, deg: f32) -> Pos2 {
    let (s, k) = wordcraft_geom::normalize_degrees(deg).to_radians().sin_cos();
    let (u, v) = (p.x - c.x, p.y - c.y);
    pos2(c.x + u * k - v * s, c.y + u * s + v * k)
}

/// One of the eight handles: `x` / `y` are -1 (left/top), 0 (middle) or 1 (right/bottom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle {
    pub x: i8,
    pub y: i8,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle { x: -1, y: -1 },
        Handle { x: 0, y: -1 },
        Handle { x: 1, y: -1 },
        Handle { x: 1, y: 0 },
        Handle { x: 1, y: 1 },
        Handle { x: 0, y: 1 },
        Handle { x: -1, y: 1 },
        Handle { x: -1, y: 0 },
    ];
    pub fn is_corner(self) -> bool {
        self.x != 0 && self.y != 0
    }
    pub fn cursor(self) -> CursorIcon {
        match (self.x, self.y) {
            (0, _) => CursorIcon::ResizeVertical,
            (_, 0) => CursorIcon::ResizeHorizontal,
            (a, b) if a == b => CursorIcon::ResizeNwSe,
            _ => CursorIcon::ResizeNeSw,
        }
    }
    fn at(self, f: egui::Rect) -> Pos2 {
        let k = |v: i8| (f32::from(v) + 1.0) / 2.0;
        pos2(f.min.x + f.width() * k(self.x), f.min.y + f.height() * k(self.y))
    }
    /// Where it is on frame `f` turned `deg`.
    fn at_turned(self, f: egui::Rect, deg: f32) -> Pos2 {
        turn(self.at(f), f.center(), deg)
    }
}

/// Where the rotation handle of frame `f` turned `deg` is: above the middle of its top edge.
fn rotate_handle(f: egui::Rect, deg: f32) -> Pos2 {
    turn(pos2(f.center().x, f.min.y - ROTATE_GAP), f.center(), deg)
}

/// Whether screen point `p` is on the rotation handle of frame `f` turned `deg`.
pub fn rotate_handle_at(f: egui::Rect, deg: f32, p: Pos2) -> bool {
    rotate_handle(f, deg).distance(p) <= GRAB_R
}

/// What a press grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    Move,
    Resize(Handle),
    /// The rotation handle.
    Rotate,
}

impl Grab {
    pub fn cursor(self) -> CursorIcon {
        match self {
            Grab::Move => CursorIcon::Move,
            Grab::Resize(h) => h.cursor(),
            Grab::Rotate => CursorIcon::Crosshair,
        }
    }
}

/// A page rect (points) on screen, given the page's screen rect and the scale.
pub fn to_screen(page: egui::Rect, scale: f32, r: Rect) -> egui::Rect {
    egui::Rect::from_min_size(pos2(page.min.x + r.x * scale, page.min.y + r.y * scale), vec2(r.w * scale, r.h * scale))
}

/// The handles a frame shows: corners always, edge middles when there's room.
fn handles(f: egui::Rect) -> impl Iterator<Item = Handle> {
    let (wide, tall) = (f.width() >= EDGE_HANDLES_FROM, f.height() >= EDGE_HANDLES_FROM);
    Handle::ALL.into_iter().filter(move |h| (h.x != 0 || wide) && (h.y != 0 || tall))
}

/// The handle of frame `f` turned `deg` under screen point `p`.
pub fn handle_at(f: egui::Rect, deg: f32, p: Pos2) -> Option<Handle> {
    let d = |h: &Handle| h.at_turned(f, deg).distance(p);
    handles(f).filter(|h| d(h) <= GRAB_R).min_by(|a, b| d(a).total_cmp(&d(b)))
}

/// The corners of frame `f` turned `deg`, clockwise from the top left, closed.
fn outline(f: egui::Rect, deg: f32) -> [Pos2; 5] {
    let c = f.center();
    [f.left_top(), f.right_top(), f.right_bottom(), f.left_bottom(), f.left_top()].map(|p| turn(p, c, deg))
}

/// Draw a frame turned `deg`: its outline (dashed while what's inside is being edited) and,
/// unless it's being dragged, its handles — `rotate` adds the rotation handle above it.
pub fn paint(painter: &Painter, t: &Tokens, f: egui::Rect, deg: f32, dashed: bool, with_handles: bool, rotate: bool) {
    let stroke = Stroke::new(1.25, t.accent);
    let c = outline(f, deg);
    if dashed {
        painter.extend(egui::Shape::dashed_line(&c, stroke, 4.0, 3.0));
    } else if wordcraft_geom::normalize_degrees(deg) == 0.0 {
        painter.rect_stroke(f, 0.0, stroke, egui::StrokeKind::Outside);
    } else {
        painter.add(egui::Shape::line(c.to_vec(), stroke));
    }
    if !with_handles {
        return;
    }
    if rotate {
        let r = rotate_handle(f, deg);
        painter.circle_filled(r + vec2(0.0, 0.75), HANDLE_R + 2.0, t.handle_shadow);
        painter.circle(r, HANDLE_R + 1.0, t.handle, Stroke::new(1.25, t.accent));
        // A circling arrow: three quarters of a ring and its head.
        let (rr, k) = (HANDLE_R * 0.55, std::f32::consts::PI / 12.0);
        let ring: Vec<Pos2> = (0..=10).map(|i| r + vec2((k * (i as f32 * 1.8 - 6.0)).cos(), (k * (i as f32 * 1.8 - 6.0)).sin()) * rr).collect();
        if let Some(end) = ring.last().copied() {
            painter.add(egui::Shape::line(ring, Stroke::new(1.0, t.accent)));
            painter.circle_filled(end, 1.3, t.accent);
        }
    }
    for h in handles(f) {
        let c = h.at_turned(f, deg);
        painter.circle_filled(c + vec2(0.0, 0.75), HANDLE_R + 1.0, t.handle_shadow);
        painter.circle(c, HANDLE_R, t.handle, Stroke::new(1.25, t.accent));
    }
}

/// Where a frame turned `deg` is being dragged to: its outline, faintly filled.
pub fn paint_outline(painter: &Painter, t: &Tokens, f: egui::Rect, deg: f32) {
    let c = outline(f, deg);
    painter.add(egui::Shape::convex_polygon(c[..4].to_vec(), t.accent.gamma_multiply(0.08), Stroke::new(1.25, t.accent)));
}

/// A frame being turned to `deg`: its outline and the angle beside the rotation handle.
pub fn paint_turning(painter: &Painter, t: &Tokens, f: egui::Rect, deg: f32) {
    paint_outline(painter, t, f, deg);
    let label = format!("{:.0}°", wordcraft_geom::normalize_degrees(deg.round()));
    painter.text(rotate_handle(f, deg) + vec2(HANDLE_R + 6.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(11.0), t.accent);
}

/// A drag of a frame in progress.
#[derive(Clone, Debug)]
pub struct Drag {
    pub grab: Grab,
    /// The page the frame started on and its rect there (points).
    pub page: usize,
    pub start: Rect,
    /// The press, screen.
    press: Pos2,
    /// Where it would land: page and rect (points).
    pub to_page: usize,
    pub rect: Rect,
    /// Past the drag threshold (a press that never gets there is a click).
    pub moved: bool,
    /// The frame's angle (degrees clockwise): as it started, or where turning puts it.
    pub deg: f32,
}

impl Drag {
    /// A drag of the frame at `start` on `page`, turned `deg`, grabbed at screen point `press`.
    pub fn new(grab: Grab, page: usize, start: Rect, deg: f32, press: Pos2) -> Drag {
        Drag { grab, page, start, press, to_page: page, rect: start, moved: false, deg: wordcraft_geom::normalize_degrees(deg) }
    }

    /// Follow the pointer with the rotation handle: the frame turns to face it, by whole degrees
    /// (or `snap`ping to 15° steps, Shift).
    pub fn turn_to(&mut self, pointer: Pos2, pages: &[egui::Rect], scale: f32, snap: bool) {
        if !self.moved && pointer.distance(self.press) < DRAG_START {
            return;
        }
        let Some(page) = pages.get(self.page) else { return };
        self.moved = true;
        let c = to_screen(*page, scale, self.start).center();
        let (dx, dy) = (pointer.x - c.x, pointer.y - c.y);
        if dx.hypot(dy) < 1.0 {
            return;
        }
        // The handle is straight up (-90°) when unturned.
        let deg = dy.atan2(dx).to_degrees() + 90.0;
        let step = if snap { SNAP_DEG } else { 1.0 };
        self.deg = wordcraft_geom::normalize_degrees((deg / step).round() * step);
    }

    /// Follow the pointer. A move keeps the grabbed point under it, across pages too; a resize
    /// moves the grabbed edges, keeping `min` size and, with `keep_aspect`, the proportions.
    pub fn update(&mut self, pointer: Pos2, pages: &[egui::Rect], scale: f32, min: f32, keep_aspect: bool) {
        if !self.moved && pointer.distance(self.press) < DRAG_START {
            return;
        }
        self.moved = true;
        let scale = scale.max(0.01);
        match self.grab {
            Grab::Move => {
                let Some(origin) = pages.get(self.page) else { return };
                let Some((page, r)) = crate::canvas::nearest_page(pages, pointer).and_then(|i| Some((i, pages.get(i)?))) else { return };
                let (x, y) = ((pointer.x - r.min.x) / scale, (pointer.y - r.min.y) / scale);
                // The grabbed point within the frame, points.
                let gx = (self.press.x - origin.min.x) / scale - self.start.x;
                let gy = (self.press.y - origin.min.y) / scale - self.start.y;
                self.to_page = page;
                self.rect = Rect::new(x - gx, y - gy, self.start.w, self.start.h);
            }
            Grab::Resize(h) => {
                let d = (pointer - self.press) / scale;
                self.to_page = self.page;
                self.rect = resized_turned(self.start, self.deg, h, d.x, d.y, min, keep_aspect);
            }
            Grab::Rotate => {}
        }
    }
}

/// [`resized`] for a frame turned `deg`: the drag is taken along the frame's own axes, and the
/// handle opposite the grabbed one stays where it is on the page.
pub fn resized_turned(r: Rect, deg: f32, h: Handle, dx: f32, dy: f32, min: f32, keep_aspect: bool) -> Rect {
    if wordcraft_geom::normalize_degrees(deg) == 0.0 {
        return resized(r, h, dx, dy, min, keep_aspect);
    }
    let o = pos2(0.0, 0.0);
    let d = turn(pos2(dx, dy), o, -deg);
    let n = resized(r, h, d.x, d.y, min, keep_aspect);
    // The opposite handle is at the same frame point in both rects; turned about different
    // centres it would move, so shift the new rect back under it.
    let opp = Handle { x: -h.x, y: -h.y };
    let er = |r: Rect| egui::Rect::from_min_size(pos2(r.x, r.y), vec2(r.w, r.h));
    let (was, now) = (opp.at_turned(er(r), deg), opp.at_turned(er(n), deg));
    Rect::new(n.x + was.x - now.x, n.y + was.y - now.y, n.w, n.h)
}

/// `r` with handle `h` dragged by (dx, dy) points; the opposite edges stay put. Never smaller
/// than `min`; `keep_aspect` keeps the proportions when a corner is dragged.
pub fn resized(r: Rect, h: Handle, dx: f32, dy: f32, min: f32, keep_aspect: bool) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (r.x, r.y, r.right(), r.bottom());
    match h.x {
        -1 => x0 = (x0 + dx).min(x1 - min),
        1 => x1 = (x1 + dx).max(x0 + min),
        _ => {}
    }
    match h.y {
        -1 => y0 = (y0 + dy).min(y1 - min),
        1 => y1 = (y1 + dy).max(y0 + min),
        _ => {}
    }
    if keep_aspect && h.is_corner() && r.w > 0.0 && r.h > 0.0 {
        // Follow whichever side changed more.
        let (kx, ky) = ((x1 - x0) / r.w, (y1 - y0) / r.h);
        let k = if (kx - 1.0).abs() >= (ky - 1.0).abs() { kx } else { ky };
        let k = k.max(min / r.w).max(min / r.h);
        let (w, hh) = (r.w * k, r.h * k);
        if h.x < 0 {
            x0 = x1 - w;
        } else {
            x1 = x0 + w;
        }
        if h.y < 0 {
            y0 = y1 - hh;
        } else {
            y1 = y0 + hh;
        }
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect { x: 100.0, y: 50.0, w: 200.0, h: 100.0 };

    #[test]
    fn resize_moves_only_the_grabbed_edges() {
        assert_eq!(resized(R, Handle { x: 1, y: 0 }, 30.0, 99.0, 4.0, false), Rect::new(100.0, 50.0, 230.0, 100.0));
        assert_eq!(resized(R, Handle { x: -1, y: -1 }, 20.0, 10.0, 4.0, false), Rect::new(120.0, 60.0, 180.0, 90.0));
        assert_eq!(resized(R, Handle { x: 0, y: 1 }, 50.0, -40.0, 4.0, false), Rect::new(100.0, 50.0, 200.0, 60.0));
    }

    #[test]
    fn resize_keeps_min_size_and_the_far_edge() {
        let r = resized(R, Handle { x: -1, y: 0 }, 500.0, 0.0, 18.0, false);
        assert_eq!((r.right(), r.w), (R.right(), 18.0));
        let r = resized(R, Handle { x: 1, y: 1 }, -1000.0, -1000.0, 18.0, true);
        assert!(r.w >= 18.0 && r.h >= 18.0 && (r.w / r.h - 2.0).abs() < 1e-3, "{r:?}");
        assert_eq!((r.x, r.y), (R.x, R.y));
    }

    #[test]
    fn corner_resize_can_keep_proportions() {
        let r = resized(R, Handle { x: 1, y: 1 }, 100.0, 0.0, 4.0, true);
        assert_eq!(r, Rect::new(100.0, 50.0, 300.0, 150.0));
        let r = resized(R, Handle { x: -1, y: -1 }, 0.0, 50.0, 4.0, true);
        assert_eq!(r, Rect::new(200.0, 100.0, 100.0, 50.0));
    }

    #[test]
    fn handles_and_hits() {
        let f = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 20.0));
        // Too short for side handles; wide enough for top/bottom middles.
        let shown: Vec<Handle> = handles(f).collect();
        assert_eq!(shown.len(), 6);
        assert_eq!(handle_at(f, 0.0, pos2(101.0, 21.0)), Some(Handle { x: 1, y: 1 }));
        assert_eq!(handle_at(f, 0.0, pos2(50.0, -2.0)), Some(Handle { x: 0, y: -1 }));
        assert_eq!(handle_at(f, 0.0, pos2(50.0, 10.0)), None);
        // Turned 90° clockwise, the top-left corner is at the top right.
        assert_eq!(handle_at(f, 90.0, pos2(60.0, -40.0)), Some(Handle { x: -1, y: -1 }));
        assert!(rotate_handle_at(f, 0.0, pos2(50.0, -22.0)) && !rotate_handle_at(f, 90.0, pos2(50.0, -22.0)));
    }

    #[test]
    fn moving_keeps_the_grab_point_under_the_pointer() {
        let pages =
            [egui::Rect::from_min_size(pos2(10.0, 10.0), vec2(612.0, 792.0)), egui::Rect::from_min_size(pos2(10.0, 812.0), vec2(612.0, 792.0))];
        let mut d = Drag::new(Grab::Move, 0, R, 0.0, pos2(10.0 + 150.0, 10.0 + 60.0));
        d.update(pos2(161.0, 71.0), &pages, 1.0, 4.0, false);
        assert!(!d.moved, "under the drag threshold");
        d.update(pos2(210.0, 110.0), &pages, 1.0, 4.0, false);
        assert!(d.moved);
        assert_eq!((d.to_page, d.rect), (0, Rect::new(150.0, 90.0, 200.0, 100.0)));
        // Onto the next page.
        d.update(pos2(160.0, 812.0 + 60.0), &pages, 1.0, 4.0, false);
        assert_eq!((d.to_page, d.rect), (1, R));
    }

    #[test]
    fn the_rotation_handle_turns_the_frame_and_shift_snaps() {
        let pages = [egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(612.0, 792.0))];
        // R's centre is (200, 100); its rotation handle is above it.
        let mut d = Drag::new(Grab::Rotate, 0, R, 0.0, pos2(200.0, 28.0));
        d.turn_to(pos2(300.0, 100.0), &pages, 1.0, false);
        assert_eq!(d.deg, 90.0);
        d.turn_to(pos2(200.0 + 100.0, 100.0 - 60.0), &pages, 1.0, false);
        assert_eq!(d.deg, 59.0);
        d.turn_to(pos2(200.0 + 100.0, 100.0 - 60.0), &pages, 1.0, true);
        assert_eq!(d.deg, 60.0);
        d.turn_to(pos2(100.0, 100.0), &pages, 1.0, true);
        assert_eq!(d.deg, 270.0);
    }

    #[test]
    fn a_turned_frame_resizes_along_its_own_axes() {
        // Turned 90°: dragging the right handle (now at the bottom) down widens the frame, and
        // its left edge (now at the top) stays put on the page.
        let r = resized_turned(R, 90.0, Handle { x: 1, y: 0 }, 0.0, 40.0, 4.0, false);
        assert!((r.w - 240.0).abs() < 1e-3 && (r.h - 100.0).abs() < 1e-3, "{r:?}");
        let top = |r: Rect| turn(pos2(r.x, r.y + r.h / 2.0), pos2(r.x + r.w / 2.0, r.y + r.h / 2.0), 90.0);
        assert!(top(r).distance(top(R)) < 1e-3, "{:?} {:?}", top(r), top(R));
    }
}

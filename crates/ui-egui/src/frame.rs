//! A transform frame: the outline and handles around something on a page that is moved by
//! dragging it and resized by dragging a handle. It knows nothing about what it frames: callers
//! give a page rect (points) and get back where the drag puts it, so pictures, shapes, text boxes
//! and anything else with a page rect share it.

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
}

/// What a press grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    Move,
    Resize(Handle),
}

impl Grab {
    pub fn cursor(self) -> CursorIcon {
        match self {
            Grab::Move => CursorIcon::Move,
            Grab::Resize(h) => h.cursor(),
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

/// The handle of frame `f` under screen point `p`.
pub fn handle_at(f: egui::Rect, p: Pos2) -> Option<Handle> {
    handles(f).filter(|h| h.at(f).distance(p) <= GRAB_R).min_by(|a, b| a.at(f).distance(p).total_cmp(&b.at(f).distance(p)))
}

/// Draw a frame: its outline (dashed while what's inside is being edited) and, unless it's being
/// dragged, its handles.
pub fn paint(painter: &Painter, t: &Tokens, f: egui::Rect, dashed: bool, with_handles: bool) {
    let stroke = Stroke::new(1.25, t.accent);
    if dashed {
        let c = [f.left_top(), f.right_top(), f.right_bottom(), f.left_bottom(), f.left_top()];
        painter.extend(egui::Shape::dashed_line(&c, stroke, 4.0, 3.0));
    } else {
        painter.rect_stroke(f, 0.0, stroke, egui::StrokeKind::Outside);
    }
    if !with_handles {
        return;
    }
    for h in handles(f) {
        let c = h.at(f);
        painter.circle_filled(c + vec2(0.0, 0.75), HANDLE_R + 1.0, t.handle_shadow);
        painter.circle(c, HANDLE_R, t.handle, Stroke::new(1.25, t.accent));
    }
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
}

impl Drag {
    pub fn new(grab: Grab, page: usize, start: Rect, press: Pos2) -> Drag {
        Drag { grab, page, start, press, to_page: page, rect: start, moved: false }
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
                self.rect = resized(self.start, h, d.x, d.y, min, keep_aspect);
            }
        }
    }
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
        assert_eq!(handle_at(f, pos2(101.0, 21.0)), Some(Handle { x: 1, y: 1 }));
        assert_eq!(handle_at(f, pos2(50.0, -2.0)), Some(Handle { x: 0, y: -1 }));
        assert_eq!(handle_at(f, pos2(50.0, 10.0)), None);
    }

    #[test]
    fn moving_keeps_the_grab_point_under_the_pointer() {
        let pages =
            [egui::Rect::from_min_size(pos2(10.0, 10.0), vec2(612.0, 792.0)), egui::Rect::from_min_size(pos2(10.0, 812.0), vec2(612.0, 792.0))];
        let mut d = Drag::new(Grab::Move, 0, R, pos2(10.0 + 150.0, 10.0 + 60.0));
        d.update(pos2(161.0, 71.0), &pages, 1.0, 4.0, false);
        assert!(!d.moved, "under the drag threshold");
        d.update(pos2(210.0, 110.0), &pages, 1.0, 4.0, false);
        assert!(d.moved);
        assert_eq!((d.to_page, d.rect), (0, Rect::new(150.0, 90.0, 200.0, 100.0)));
        // Onto the next page.
        d.update(pos2(160.0, 812.0 + 60.0), &pages, 1.0, 4.0, false);
        assert_eq!((d.to_page, d.rect), (1, R));
    }
}

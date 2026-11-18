//! A short, eased motion for the selected object when a command changes its place, size or
//! angle, and a faint preview of the target size while a handle is dragged.
//!
//! This is only a picture drawn over the page. The document and its layout are never touched:
//! the object is already at its final place, so a motion that is cut short, turned off or never
//! drawn leaves nothing wrong behind. While it goes the page is drawn without the object (as for
//! a drag) and the object, drawn alone as it now is, is carried from its old pose to its new one,
//! so it ends exactly where the page then shows it.
//!
//! While a handle or the object is dragged, the original and the target are two faint copies.
//! On release the original sets off to the target, both still faint, and the solid
//! object takes over when it gets there. A command does the same without the drag.
//!
//! Nothing here knows about shapes. A subject is a [`Pose`] (page, frame, angle) under a
//! document position, so pictures and other objects can join by being offered by
//! [`crate::objects::motion_subject`].
//!
//! No motion when: animation effects are off (File ▸ Options ▸ Advanced; the system's
//! reduced-motion setting counts only while that is "Use system setting"), egui's
//! `animation_time` is 0 (the headless harness and `ui_shot`), or the picture of the object
//! would not fit a GPU texture.

use egui::{Color32, Pos2};
use wordcraft_doc::Pos;
use wordcraft_geom::Rect;

use crate::WordApp;

/// How long a motion takes (seconds).
pub const SECS: f64 = 0.16;
/// A pose that differs by less than this (points, degrees) has not moved.
const EPS: f32 = 0.05;

/// Where an object is: its page, its unrotated frame (page points) and its angle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub page: usize,
    pub rect: Rect,
    pub deg: f32,
}

impl Pose {
    fn same(&self, o: &Pose) -> bool {
        self.page == o.page
            && (self.rect.x - o.rect.x).abs() < EPS
            && (self.rect.y - o.rect.y).abs() < EPS
            && (self.rect.w - o.rect.w).abs() < EPS
            && (self.rect.h - o.rect.h).abs() < EPS
            && turn(self.deg, o.deg).abs() < EPS
    }
}

/// Fast at first, then settling. Out of range input is clamped; NaN counts as finished.
pub fn ease(t: f32) -> f32 {
    if t.is_nan() {
        return 1.0;
    }
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// The signed shortest turn from `from` to `to` (degrees, −180 to 180).
fn turn(from: f32, to: f32) -> f32 {
    let d = wordcraft_geom::normalize_degrees(to - from);
    if d > 180.0 { d - 360.0 } else { d }
}

/// A move from one pose to another, started at `start` (egui's clock, seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    pub from: Pose,
    pub to: Pose,
    pub start: f64,
    pub secs: f64,
}

impl Tween {
    /// How far along it is, 0 to 1 (eased).
    pub fn progress(&self, now: f64) -> f32 {
        if !self.secs.is_finite() || self.secs <= 0.0 || !now.is_finite() {
            return 1.0;
        }
        ease(((now - self.start) / self.secs) as f32)
    }

    pub fn done(&self, now: f64) -> bool {
        !self.secs.is_finite() || self.secs <= 0.0 || !now.is_finite() || now - self.start >= self.secs
    }

    /// The pose at `now`; the angle takes the short way round.
    pub fn pose(&self, now: f64) -> Pose {
        let t = self.progress(now);
        let l = |a: f32, b: f32| a + (b - a) * t;
        let (a, b) = (self.from, self.to);
        Pose {
            page: b.page,
            rect: Rect::new(l(a.rect.x, b.rect.x), l(a.rect.y, b.rect.y), l(a.rect.w, b.rect.w), l(a.rect.h, b.rect.h)),
            deg: wordcraft_geom::normalize_degrees(a.deg + turn(a.deg, b.deg) * t),
        }
    }
}

/// Where the four corners of a picture of an object (taken at `base`, covering `bounds`, the
/// frame's rotated bounds) go when the object is at `at`: the picture is stretched along the
/// frame's own axes and turned with it. Order: top left, top right, bottom right, bottom left.
pub fn corners(base: &Pose, bounds: Rect, at: &Pose) -> [(f32, f32); 4] {
    let c0 = (base.rect.x + base.rect.w / 2.0, base.rect.y + base.rect.h / 2.0);
    let c1 = (at.rect.x + at.rect.w / 2.0, at.rect.y + at.rect.h / 2.0);
    let ratio = |new: f32, old: f32| if old > 0.001 && new.is_finite() { new / old } else { 1.0 };
    let (sx, sy) = (ratio(at.rect.w, base.rect.w), ratio(at.rect.h, base.rect.h));
    let (s0, c_0) = (-base.deg.to_radians()).sin_cos();
    let (s1, c_1) = at.deg.to_radians().sin_cos();
    let at_corner = |x: f32, y: f32| {
        let (vx, vy) = (x - c0.0, y - c0.1);
        let (lx, ly) = ((vx * c_0 - vy * s0) * sx, (vx * s0 + vy * c_0) * sy);
        (c1.0 + lx * c_1 - ly * s1, c1.1 + lx * s1 + ly * c_1)
    };
    let b = bounds;
    [at_corner(b.x, b.y), at_corner(b.x + b.w, b.y), at_corner(b.x + b.w, b.y + b.h), at_corner(b.x, b.y + b.h)]
}

/// The object as it looked before the last change, drawn alone: the faint picture of a resize.
#[derive(Clone)]
struct Seen {
    key: Pos,
    rev: u64,
    pose: Pose,
    bounds: Rect,
    tex: Option<egui::TextureHandle>,
}

/// A motion in flight: the object as it looks now (drawn alone, taken at `end`) carried along
/// `tween` to where it is. The page is drawn without the object meanwhile ([`hidden`]).
#[derive(Clone)]
struct Live {
    page_hidden: (usize, Pos),
    tween: Tween,
    end: Pose,
    bounds: Rect,
    tex: egui::TextureHandle,
    /// Made this frame, which laid the page out again and drew it without the object: that time
    /// is not part of the motion, so the clock starts with the next frame.
    fresh: bool,
    /// When it was last drawn: a motion cut short goes on from the pose drawn then.
    drawn: f64,
}

#[derive(Clone, Default)]
struct State {
    document: u64,
    seen: Option<Seen>,
    live: Option<Live>,
    /// The object a drag has taken off its page: its original is drawn as a faint copy.
    dragged: Option<(usize, Pos)>,
}

/// How strong the copy of the original is, while the object is dragged and on its way.
const WAS: f32 = 0.35;
/// How strong the copy at the target is.
const TARGET: f32 = 0.45;

fn state_id() -> egui::Id {
    egui::Id::unique("object_motion")
}

fn quad(tex: &egui::TextureHandle, pts: [Pos2; 4], alpha: f32) -> egui::Shape {
    let color = Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8);
    let mut mesh = egui::Mesh::with_texture(tex.id());
    let uvs = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    for (p, (u, v)) in pts.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: egui::pos2(u, v), color });
    }
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    egui::Shape::mesh(mesh)
}

fn on_screen(page: egui::Rect, scale: f32, c: [(f32, f32); 4]) -> [Pos2; 4] {
    c.map(|(x, y)| egui::pos2(page.min.x + x * scale, page.min.y + y * scale))
}

/// The picture of an object this size is a texture the GPU takes (a huge or hostile frame gets
/// no motion).
fn fits(ctx: &egui::Context, b: Rect, scale: f32) -> bool {
    let k = scale * ctx.pixels_per_point();
    let side = ctx.input(|i| i.max_texture_side) as f32;
    let (w, h) = (b.w * k, b.h * k);
    w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0 && w <= side && h <= side && w * h <= 16.0e6
}

/// The object a motion is drawing, which the page is drawn without while it goes.
pub fn hidden(ctx: &egui::Context) -> Option<(usize, Pos)> {
    ctx.data(|d| d.get_temp::<State>(state_id())).and_then(|s| s.live.map(|l| l.page_hidden).or(s.dragged))
}

/// A motion is being drawn (for tests and tools).
pub fn in_flight(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<State>(state_id())).is_some_and(|s| s.live.is_some())
}

/// Called each frame by the canvas before it draws the pages: notices a change of pose caused by
/// a command (or by letting go of a handle) and starts the motion.
pub fn begin(app: &WordApp, ctx: &egui::Context, layout: &wordcraft_layout::DocLayout, scale: f32) {
    let id = state_id();
    if !app.animations_on() || ctx.global_style().animation_time <= 0.0 || SECS <= 0.0 {
        ctx.data_mut(|d| d.remove::<State>(id));
        return;
    }
    let mut st = ctx.data(|d| d.get_temp::<State>(id)).unwrap_or_default();
    let now = ctx.input(|i| i.time);
    let doc = app.session.document_id();
    if st.document != doc {
        st = State { document: doc, ..State::default() };
    }
    if let Some(l) = st.live.as_mut().filter(|l| l.fresh) {
        l.tween.start = now;
        l.fresh = false;
    }
    if st.live.as_ref().is_some_and(|l| l.tween.done(now)) {
        st.live = None;
    }
    // A shape deleted while it moves leaves no copies behind.
    if st.live.as_ref().is_some_and(|l| layout.object(&l.page_hidden.1, l.page_hidden.0).is_none()) {
        st.live = None;
    }

    if let Some(drag) = crate::objects::motion_drag(app) {
        // Only an object with a picture of its original can be taken off its page.
        // A turn of a box with no pixels of its own to show keeps the solid original in place.
        let shows = drag.pixels || drag.resize;
        let known = shows && st.seen.as_ref().is_some_and(|s| s.key == drag.key && s.tex.is_some());
        st.dragged = known.then_some((drag.page, drag.key));
        st.live = None;
        ctx.data_mut(|d| d.insert_temp(id, st));
        return;
    }
    // Letting go of a handle or of the object itself (a move, a resize, a turn) sets the faint
    // copy of the original off to where the target copy showed.
    st.dragged = None;

    let rev = app.session.rev();
    let subject = crate::objects::motion_subject(app, layout);
    let current = subject.as_ref().map(|o| (o.pos(), Pose { page: o.page, rect: o.rect, deg: o.spin.deg }, o.bounds()));
    match (current, st.seen.take()) {
        // Moving a shape to another page anchors it to another paragraph: its position then
        // changes. It is the same object when the old position holds none any more and the
        // frame has the same size (a deleted shape with another selected is not a move).
        (Some((key, pose, bounds)), Some(seen))
            if seen.key == key
                || (seen.rev != rev
                    && (seen.pose.rect.w - pose.rect.w).abs() < EPS
                    && (seen.pose.rect.h - pose.rect.h).abs() < EPS
                    && layout.object(&seen.key, seen.pose.page).is_none()) =>
        {
            if seen.rev == rev && seen.pose.same(&pose) {
                st.seen = Some(seen);
            } else {
                // The old motion ends here. If the new one cannot be drawn (the object is too
                // big for a picture), the solid object shows at once.
                let old = if seen.pose.same(&pose) { None } else { st.live.take() };
                if !seen.pose.same(&pose)
                    && fits(ctx, bounds, scale)
                    && let Some(o) = &subject
                    && let Some(tex) = crate::objects::preview(app, ctx, layout, o, scale)
                {
                    // A motion cut short by another change goes on from where it was drawn.
                    let from = old.map_or(seen.pose, |l| l.tween.pose(l.drawn.min(now)));
                    st.live = Some(Live {
                        page_hidden: (o.page, key.clone()),
                        tween: Tween { from, to: pose, start: now, secs: SECS },
                        end: pose,
                        bounds,
                        tex,
                        fresh: true,
                        drawn: now,
                    });
                }
                st.seen = Some(Seen { key, rev, pose, bounds, tex: None });
            }
        }
        (Some((key, pose, bounds)), _) => {
            // Another object is selected: it does not take over the motion of the first.
            if st.live.as_ref().is_some_and(|l| l.page_hidden.1 != key) {
                st.live = None;
            }
            st.seen = Some(Seen { key, rev, pose, bounds, tex: None });
        }
        (None, _) => {}
    }
    // The faint picture for a resize: the object as it is now.
    if st.live.is_none()
        && let (Some(o), Some(s)) = (&subject, st.seen.as_mut())
        && s.tex.is_none()
        && fits(ctx, s.bounds, scale)
    {
        s.tex = crate::objects::preview(app, ctx, layout, o, scale);
    }
    ctx.data_mut(|d| d.insert_temp(id, st));
}

/// Called each frame by the canvas over the pages: draws the object on its way, and the faint
/// picture of the size a handle drag leads to. `pages` are the pages' screen rects.
pub fn paint(app: &WordApp, painter: &egui::Painter, pages: &[egui::Rect], scale: f32) {
    let ctx = painter.ctx();
    let id = state_id();
    let Some(mut st) = ctx.data(|d| d.get_temp::<State>(id)) else { return };
    let now = ctx.input(|i| i.time);
    if let Some(drag) = crate::objects::motion_drag(app) {
        // The original, faint where it was (the page is drawn without it).
        if let Some(s) = st.seen.as_ref().filter(|s| s.key == drag.key && (drag.pixels || drag.resize))
            && let Some(tex) = &s.tex
            && let Some(page) = pages.get(s.pose.page)
        {
            painter.add(quad(tex, on_screen(*page, scale, corners(&s.pose, s.bounds, &s.pose)), WAS));
            // The target: a picture that is not turned already shows its pixels while dragging.
            if drag.resize
                && !drag.pixels
                && let Some(page) = pages.get(drag.to.page)
            {
                painter.add(quad(tex, on_screen(*page, scale, corners(&s.pose, s.bounds, &drag.to)), TARGET));
            }
        }
        return;
    }
    if let Some(l) = &st.live
        && !l.tween.done(now)
        && let Some(page) = pages.get(l.tween.to.page)
    {
        // Both copies stay faint until the original gets there, then the solid object takes over.
        painter.add(quad(&l.tex, on_screen(*page, scale, corners(&l.end, l.bounds, &l.end)), TARGET));
        let at = l.tween.pose(now);
        let from = l.tween.from;
        let pts = match pages.get(from.page) {
            // Text moved it to another page: the corners go across the screen between the two.
            Some(from_page) if from.page != l.tween.to.page => {
                let t = l.tween.progress(now);
                let a = on_screen(*from_page, scale, corners(&l.end, l.bounds, &from));
                let b = on_screen(*page, scale, corners(&l.end, l.bounds, &l.end));
                std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
            }
            _ => on_screen(*page, scale, corners(&l.end, l.bounds, &at)),
        };
        painter.add(quad(&l.tex, pts, WAS));
        ctx.request_repaint();
        if let Some(l) = st.live.as_mut() {
            l.drawn = now;
        }
        ctx.data_mut(|d| d.insert_temp(id, st));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32, y: f32, w: f32, h: f32, deg: f32) -> Pose {
        Pose { page: 0, rect: Rect::new(x, y, w, h), deg }
    }

    fn tween(from: Pose, to: Pose) -> Tween {
        Tween { from, to, start: 10.0, secs: 0.2 }
    }

    #[test]
    fn ease_is_monotonic_and_bounded() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert_eq!(ease(-3.0), 0.0);
        assert_eq!(ease(9.0), 1.0);
        assert_eq!(ease(f32::NAN), 1.0);
        let mut last = 0.0;
        for i in 0..=20 {
            let v = ease(i as f32 / 20.0);
            assert!(v >= last);
            last = v;
        }
        assert!(ease(0.5) > 0.5, "fast first");
    }

    #[test]
    fn ends_exactly_at_the_target() {
        let t = tween(pose(10.0, 20.0, 50.0, 40.0, 0.0), pose(100.0, 200.0, 80.0, 60.0, 90.0));
        assert_eq!(t.pose(10.0), t.from);
        assert_eq!(t.pose(10.2), t.to);
        assert_eq!(t.pose(99.0), t.to);
        assert_eq!(t.pose(f64::NAN), t.to);
        assert!(t.done(10.25) && !t.done(10.1));
    }

    #[test]
    fn turns_the_short_way() {
        let t = tween(pose(0.0, 0.0, 10.0, 10.0, 350.0), pose(0.0, 0.0, 10.0, 10.0, 10.0));
        let mid = t.pose(10.2).deg;
        assert!((mid - 10.0).abs() < 1e-3);
        let half = Tween { secs: 0.2, start: 0.0, ..t };
        // Whatever the easing, the angle stays within the 20° arc through 0.
        for i in 0..=10 {
            let d = half.pose(i as f64 * 0.02).deg;
            assert!(d >= 349.9 || d <= 10.1, "{d}");
        }
    }

    #[test]
    fn zero_or_bad_duration_finishes_at_once() {
        let mut t = tween(pose(0.0, 0.0, 1.0, 1.0, 0.0), pose(5.0, 5.0, 1.0, 1.0, 0.0));
        t.secs = 0.0;
        assert!(t.done(10.0));
        assert_eq!(t.pose(10.0), t.to);
        t.secs = f64::NAN;
        assert_eq!(t.pose(10.0), t.to);
    }

    #[test]
    fn pose_compare_ignores_noise() {
        assert!(pose(1.0, 1.0, 5.0, 5.0, 0.0).same(&pose(1.01, 1.0, 5.0, 5.0, 360.0)));
        assert!(!pose(1.0, 1.0, 5.0, 5.0, 0.0).same(&pose(1.0, 1.0, 5.0, 5.0, 90.0)));
        let mut other = pose(1.0, 1.0, 5.0, 5.0, 0.0);
        other.page = 1;
        assert!(!pose(1.0, 1.0, 5.0, 5.0, 0.0).same(&other));
    }

    #[test]
    fn corners_at_the_base_pose_are_the_bounds() {
        let p = pose(10.0, 10.0, 40.0, 20.0, 0.0);
        let c = corners(&p, Rect::new(10.0, 10.0, 40.0, 20.0), &p);
        for (got, want) in c.iter().zip([(10.0, 10.0), (50.0, 10.0), (50.0, 30.0), (10.0, 30.0)]) {
            assert!((got.0 - want.0).abs() < 1e-3 && (got.1 - want.1).abs() < 1e-3, "{got:?} {want:?}");
        }
    }

    #[test]
    fn corners_follow_a_move_a_turn_and_a_stretch() {
        let base = pose(0.0, 0.0, 40.0, 20.0, 0.0);
        let b = Rect::new(0.0, 0.0, 40.0, 20.0);
        let moved = corners(&base, b, &pose(100.0, 50.0, 40.0, 20.0, 0.0));
        assert!((moved[0].0 - 100.0).abs() < 1e-3 && (moved[0].1 - 50.0).abs() < 1e-3);
        let big = corners(&base, b, &pose(0.0, 0.0, 80.0, 40.0, 0.0));
        assert!((big[2].0 - 80.0).abs() < 1e-3 && (big[2].1 - 40.0).abs() < 1e-3);
        // A quarter turn about the centre (20, 10): the top left goes to the top right.
        let turned = corners(&base, b, &pose(0.0, 0.0, 40.0, 20.0, 90.0));
        assert!((turned[0].0 - 30.0).abs() < 1e-3 && (turned[0].1 - -10.0).abs() < 1e-3, "{turned:?}");
    }

    #[test]
    fn corners_survive_a_frame_with_no_size() {
        let base = pose(0.0, 0.0, 0.0, 0.0, 0.0);
        let c = corners(&base, Rect::new(0.0, 0.0, 0.0, 0.0), &pose(f32::NAN, 0.0, f32::INFINITY, 1.0, 0.0));
        assert_eq!(c.len(), 4);
    }
}

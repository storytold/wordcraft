//! Nib, drawn in code: an original fountain-pen nib with eyes. No artwork from
//! any other product; see CLAUDE.md's asset rule.
//!
//! Drawn in a unit box (x, y in 0–1) mapped onto the rect, so it scales with the interface. Each
//! pose is a small, time-based variation: `clock` (seconds, for the bob and the idle blink) and
//! `since` (seconds in this pose). With animations off the caller passes `clock` 0 and a large
//! `since`, which draws every pose settled and still.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

use super::BLUE;
use crate::theme::Tokens;

/// What Nib is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pose {
    /// Standing still, blinking now and then. Still, so an idle window needs no animation
    /// frames: only one at each blink (see [`idle_wait`]).
    #[default]
    Idle,
    /// Bobbing hello (the Introduction, being turned on).
    Wave,
    /// Leaning in to write a squiggle (a Tip or agent message appears).
    Write,
    /// Eyes wide, hopping (a mass delete).
    Startled,
    /// Eyes shut, "z"s rising (a long idle).
    Doze,
    /// A nib-tip bow (being hidden, "Not now").
    Bow,
}

const OUTLINE: Color32 = Color32::from_rgb(28, 52, 132);
/// The nib's shaded (right) and lit (upper-left) faces, either side of [`BLUE`].
const SHADE: Color32 = Color32::from_rgb(53, 97, 224);
const SHINE: Color32 = Color32::from_rgb(125, 157, 252);
const EYE: Color32 = Color32::from_rgb(252, 252, 250);
const PUPIL: Color32 = Color32::from_rgb(34, 34, 40);
const INK: Color32 = Color32::from_rgb(38, 64, 150);

/// Maps unit-box points onto the rect, tilted by `angle` (radians) about the nib's tip and
/// lifted by `lift` points.
struct Frame {
    rect: Rect,
    angle: f32,
    lift: f32,
}

impl Frame {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        let tip = pos2(0.5, 0.82);
        let (s, c) = self.angle.sin_cos();
        let (dx, dy) = (x - tip.x, y - tip.y);
        let (rx, ry) = (tip.x + dx * c - dy * s, tip.y + dx * s + dy * c);
        pos2(self.rect.min.x + rx * self.rect.width(), self.rect.min.y + ry * self.rect.height() - self.lift)
    }
    fn len(&self, u: f32) -> f32 {
        u * self.rect.width()
    }
}

/// Seconds between blinks, and how long a blink lasts.
const BLINK_EVERY: f32 = 4.2;
const BLINK_FOR: f32 = 0.13;

fn blinking(clock: f32) -> bool {
    clock > 0.0 && clock.rem_euclid(BLINK_EVERY) < BLINK_FOR
}

/// Seconds until the idle drawing next changes (a blink starts or ends) at `clock`.
pub fn idle_wait(clock: f32) -> f32 {
    if !clock.is_finite() || clock <= 0.0 {
        return BLINK_FOR;
    }
    let phase = clock.rem_euclid(BLINK_EVERY);
    if phase < BLINK_FOR { BLINK_FOR - phase } else { BLINK_EVERY - phase }
}

/// Paint Nib in `rect`.
pub fn paint(p: &Painter, rect: Rect, pose: Pose, clock: f32, since: f32, t: &Tokens) {
    let settle = |d: f32| (since / d).clamp(0.0, 1.0);
    let bob = (clock * 2.0).sin() * 1.5;
    let (angle, lift) = match pose {
        Pose::Idle => (0.0, 0.0),
        Pose::Doze | Pose::Wave => (0.0, bob),
        Pose::Write => (-0.32 * (1.0 - settle(2.0)).min(settle(0.25)), 0.0),
        Pose::Startled => (0.0, 6.0 * (1.0 - settle(0.45)) * (since * 14.0).sin().abs()),
        Pose::Bow => (0.45 * (since * 2.0).sin().max(0.0) * (1.0 - settle(1.6)), 0.0),
    };
    let f = Frame { rect, angle, lift };

    // Shadow on the "desk".
    p.add(Shape::ellipse_filled(pos2(rect.center().x, rect.min.y + rect.height() * 0.9), egui::vec2(f.len(0.22), f.len(0.04)), t.page_shadow));

    // Ink squiggle while writing: drawn out over a second, under the tip.
    if pose == Pose::Write {
        let n = (24.0 * settle(1.0)) as usize;
        let pts: Vec<Pos2> = (0..n.max(1))
            .map(|i| {
                let u = i as f32 / 23.0;
                pos2(rect.min.x + rect.width() * (0.18 + 0.64 * u), rect.min.y + rect.height() * (0.93 + 0.025 * (u * 18.0).sin()))
            })
            .collect();
        if pts.len() > 1 {
            p.add(Shape::line(pts, Stroke::new(1.6, INK)));
        }
    }

    // The nib, faceted like the ArtCraft mark: a lit left face, a shaded right face and a centre
    // ridge, tapering to the tip.
    let pts = |u: &[(f32, f32)]| -> Vec<Pos2> { u.iter().map(|&(x, y)| f.pt(x, y)).collect() };
    let outline = [(0.50, 0.04), (0.66, 0.08), (0.74, 0.30), (0.66, 0.54), (0.50, 0.82), (0.34, 0.54), (0.26, 0.30), (0.34, 0.08)];
    p.add(Shape::convex_polygon(pts(&outline), BLUE, Stroke::NONE));
    p.add(Shape::convex_polygon(pts(&outline[..5]), SHADE, Stroke::NONE));
    p.add(Shape::convex_polygon(pts(&[(0.50, 0.04), (0.34, 0.08), (0.26, 0.30), (0.50, 0.17)]), SHINE, Stroke::NONE));
    p.add(Shape::closed_line(pts(&outline), Stroke::new(1.8, OUTLINE)));
    p.line_segment([f.pt(0.30, 0.17), f.pt(0.70, 0.17)], Stroke::new(1.4, OUTLINE));
    p.line_segment([f.pt(0.5, 0.17), f.pt(0.5, 0.81)], Stroke::new(1.1, OUTLINE));

    // Eyes: open, blinking, wide (startled) or shut (dozing).
    let blink = pose == Pose::Idle && blinking(clock);
    let wide = pose == Pose::Startled && since < 1.6;
    for x in [0.40, 0.60] {
        let c = f.pt(x, 0.33);
        let r = f.len(if wide { 0.105 } else { 0.085 });
        if pose == Pose::Doze || blink {
            p.line_segment([c - egui::vec2(r, 0.0), c + egui::vec2(r, 0.0)], Stroke::new(1.6, PUPIL));
        } else {
            p.circle(c, r, EYE, Stroke::new(1.2, OUTLINE));
            p.circle_filled(c + egui::vec2(-f.len(0.02), f.len(0.01)), f.len(if wide { 0.03 } else { 0.042 }), PUPIL);
        }
    }

    // Mouth: a smile, or a small "o" when startled.
    if wide {
        p.circle_stroke(f.pt(0.5, 0.46), f.len(0.03), Stroke::new(1.4, PUPIL));
    } else {
        p.add(Shape::line(vec![f.pt(0.44, 0.44), f.pt(0.5, 0.475), f.pt(0.56, 0.44)], Stroke::new(1.4, PUPIL)));
    }

    // Dozing: "z"s drifting up.
    if pose == Pose::Doze {
        for (i, size) in [(0, 10.0), (1, 13.0)] {
            let rise = (clock * 0.6 + i as f32 * 0.5).rem_euclid(1.0);
            let at = pos2(rect.max.x - f.len(0.12) + i as f32 * 6.0, rect.min.y + rect.height() * (0.12 - 0.12 * rise) - 4.0 * i as f32);
            p.text(at, egui::Align2::CENTER_CENTER, "z", egui::FontId::proportional(size), t.text.gamma_multiply(1.0 - rise * 0.7));
        }
    }
}

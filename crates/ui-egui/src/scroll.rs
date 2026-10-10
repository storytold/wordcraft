//! How the document canvas scrolls (issue #122).
//!
//! egui's built-in wheel handling smooths every delta it thinks is coarse and drops the deltas of
//! a touchpad's first and last events, and Wayland compositors leave kinetic scrolling to the app
//! (GTK, Qt and browsers synthesise it after the fingers lift). So the canvas scrolls itself:
//! - touchpad (pixel) deltas move the page 1:1, with no animation;
//! - after a touchpad flick the page coasts and slows down ([`Momentum`]), on platforms whose
//!   system doesn't already send momentum events (not macOS, Windows or the web);
//! - a mouse-wheel notch scrolls about three text lines and eases in over ~0.1 s, like egui.

use egui::{Event, InputOptions, Modifiers, MouseWheelUnit, TouchPhase, Vec2};

/// Screen points one mouse-wheel notch scrolls at 100% zoom: about three lines of body text,
/// like other desktop apps (egui's default is 40).
pub const NOTCH_PX: f32 = 56.0;

/// Whether to synthesise kinetic scrolling. macOS and Windows send their own momentum events
/// (or report touchpads as line deltas), and browsers do their own; only Linux/BSD need it.
const SYNTH_MOMENTUM: bool = !cfg!(any(target_os = "macos", target_os = "ios", target_os = "windows", target_arch = "wasm32"));

/// Longest frame time trusted for velocity and decay (a stall shouldn't fling the page).
const MAX_DT: f32 = 0.1;
/// Shortest event spacing trusted for velocity (two events in one burst aren't infinitely fast).
const MIN_DT: f32 = 1.0 / 500.0;
/// Velocity is averaged over roughly this many seconds of input.
const VELOCITY_WINDOW: f32 = 0.04;
/// With no "fingers lifted" event, coast after this long without input.
const IDLE_RELEASE: f32 = 0.05;
/// Fingers that rested this long before lifting don't fling.
const STALE: f32 = 0.1;
/// Slower flicks than this (points/s) don't coast…
const MIN_FLING: f32 = 150.0;
/// …and coasting stops below this speed.
const STOP_SPEED: f32 = 20.0;
/// Fastest coast (points/s).
const MAX_SPEED: f32 = 8000.0;
/// Exponential decay rate of the coast (1/s): speed falls to 1/e every ~0.33 s.
const DECAY: f32 = 3.0;

/// Kinetic scrolling for touchpads: tracks the scroll velocity while the fingers move and keeps
/// scrolling with exponentially decaying speed after they lift. Time-based, so frame-rate
/// independent. Deltas are in screen points, positive = content moves down/right.
#[derive(Clone, Debug, Default)]
pub struct Momentum {
    velocity: Vec2,
    /// Seconds since the last fed delta.
    since_input: f32,
    /// Fingers are on the touchpad (deltas arrived and no lift yet).
    streaming: bool,
    coasting: bool,
    /// The device reports when the fingers lift, so don't guess it from a pause.
    reports_lift: bool,
}

impl Momentum {
    /// A frame's touchpad delta, `dt` seconds after the previous frame. Non-finite input is ignored.
    pub fn feed(&mut self, delta: Vec2, dt: f32) {
        if !delta.is_finite() || !dt.is_finite() {
            return;
        }
        let dt = dt.clamp(0.0, MAX_DT);
        let fresh = !self.streaming || self.since_input + dt > STALE;
        let span = if fresh { dt } else { self.since_input + dt }.clamp(MIN_DT, MAX_DT);
        let sample = delta / span;
        self.velocity = if fresh {
            sample
        } else {
            let a = 1.0 - (-span / VELOCITY_WINDOW).exp();
            self.velocity + (sample - self.velocity) * a
        };
        let speed = self.velocity.length();
        if speed > MAX_SPEED {
            self.velocity *= MAX_SPEED / speed;
        }
        self.since_input = 0.0;
        self.streaming = true;
        self.coasting = false;
    }

    /// The fingers lifted: coast if the flick was fast and recent enough.
    pub fn release(&mut self) {
        self.reports_lift = true;
        self.start_coast();
    }

    fn start_coast(&mut self) {
        let fling = self.streaming && self.since_input <= STALE && self.velocity.length() >= MIN_FLING;
        self.streaming = false;
        if fling {
            self.coasting = true;
        } else {
            self.cancel();
        }
    }

    /// Stop dead (a click, a key, a wheel notch, the end of the document).
    pub fn cancel(&mut self) {
        self.velocity = Vec2::ZERO;
        self.coasting = false;
        self.streaming = false;
    }

    /// Advance a frame without touchpad input; returns how far to coast this frame.
    pub fn tick(&mut self, dt: f32) -> Vec2 {
        if !dt.is_finite() || dt <= 0.0 {
            return Vec2::ZERO;
        }
        let dt = dt.min(MAX_DT);
        self.since_input += dt;
        if self.streaming && !self.reports_lift && self.since_input >= IDLE_RELEASE {
            self.start_coast();
        }
        if !self.coasting {
            return Vec2::ZERO;
        }
        // Exact integral of v·e^(−kt) over the frame, so the distance doesn't depend on frame rate.
        let keep = (-DECAY * dt).exp();
        let step = self.velocity * ((1.0 - keep) / DECAY);
        self.velocity *= keep;
        if self.velocity.length() < STOP_SPEED {
            self.cancel();
        }
        step
    }

    /// Still moving or waiting to decide whether to coast: keep repainting.
    pub fn is_active(&self) -> bool {
        self.coasting || (self.streaming && !self.reports_lift)
    }
}

/// The canvas's scrolling state: momentum plus the not-yet-scrolled part of wheel notches.
#[derive(Clone, Debug, Default)]
pub struct CanvasScroll {
    momentum: Momentum,
    /// Wheel distance still to scroll, eased out over a few frames.
    pending: Vec2,
}

impl CanvasScroll {
    /// This frame's scroll delta in screen points (positive = content moves down/right) from the
    /// wheel/touchpad events in `input`. `hovered`: the pointer is over the canvas (only then do
    /// its wheel events scroll it). `notch`: points per wheel notch. `page`: viewport height.
    pub fn frame(&mut self, input: &egui::InputState, opts: &InputOptions, hovered: bool, notch: f32, page: f32) -> Vec2 {
        let notch = if notch.is_finite() { notch.clamp(1.0, 1000.0) } else { NOTCH_PX };
        let page = if page.is_finite() { page.clamp(0.0, 100_000.0) } else { 0.0 };
        // Browsers report lines (Firefox: three per notch) where native platforms report notches.
        let line = if cfg!(target_arch = "wasm32") { notch / 3.0 } else { notch };
        let dt = input.unstable_dt;
        let mut now = Vec2::ZERO;
        let mut touchpad = Vec2::ZERO;
        let mut lifted = false;
        let mut stop = false;
        for event in &input.events {
            match event {
                Event::MouseWheel { unit, delta, phase, modifiers } => {
                    // Ctrl/⌘+wheel zooms (egui turns it into a zoom factor).
                    if !hovered || modifiers.matches_any(opts.zoom_modifier) {
                        continue;
                    }
                    let d = axis_lock(if delta.is_finite() { *delta } else { Vec2::ZERO }, *modifiers, opts);
                    match unit {
                        // Touchpads (and precise mice): 1:1. Browsers also report wheel notches in
                        // points, so big jumps there still ease in, like egui does.
                        MouseWheelUnit::Point if !cfg!(target_arch = "wasm32") || d.length() < 8.0 => touchpad += d,
                        MouseWheelUnit::Point => self.pending += d,
                        // Whole notches ease in; fractional ones (X11/Windows touchpads, free-spinning
                        // wheels) are already smooth.
                        MouseWheelUnit::Line if d.x.fract() == 0.0 && d.y.fract() == 0.0 => {
                            stop = true;
                            self.pending += d * line;
                        }
                        MouseWheelUnit::Line => now += d * line,
                        MouseWheelUnit::Page => self.pending += d * page,
                    }
                    lifted |= matches!(phase, TouchPhase::End | TouchPhase::Cancel);
                }
                Event::Key { pressed: true, .. } | Event::PointerButton { pressed: true, .. } | Event::Text(_) => stop = true,
                _ => {}
            }
        }
        now += touchpad;
        let fed = touchpad != Vec2::ZERO;
        if SYNTH_MOMENTUM {
            if fed {
                self.momentum.feed(touchpad, dt);
            }
            if lifted {
                self.momentum.release();
            }
        }
        // A click, key or wheel notch stops a coast (even one that started this frame).
        if stop {
            self.momentum.cancel();
        }
        if SYNTH_MOMENTUM && !fed {
            now += self.momentum.tick(dt);
        }
        // Ease wheel notches in: 90% of the way in 0.1 s (egui's curve).
        if self.pending != Vec2::ZERO {
            let t = if dt.is_finite() { egui::emath::exponential_smooth_factor(0.9, 0.1, dt.clamp(0.0, MAX_DT)) } else { 1.0 };
            for d in 0..2 {
                let step = if self.pending[d].abs() < 1.0 { self.pending[d] } else { self.pending[d] * t };
                now[d] += step;
                self.pending[d] -= step;
            }
        }
        now
    }

    /// The scroll ran into the end of the document: stop coasting and drop queued notches.
    pub fn hit_edge(&mut self) {
        self.momentum.cancel();
        self.pending = Vec2::ZERO;
    }

    /// Scrolling continues without new input: keep repainting.
    pub fn is_animating(&self) -> bool {
        self.pending != Vec2::ZERO || self.momentum.is_active()
    }
}

/// Shift scrolls sideways and Alt scrolls vertically, like egui's own scroll areas.
fn axis_lock(d: Vec2, modifiers: Modifiers, opts: &InputOptions) -> Vec2 {
    let horizontal = modifiers.matches_any(opts.horizontal_scroll_modifier);
    let vertical = modifiers.matches_any(opts.vertical_scroll_modifier);
    match (horizontal, vertical) {
        (true, false) => Vec2::new(d.x + d.y, 0.0),
        (false, true) => Vec2::new(0.0, d.x + d.y),
        _ => d,
    }
}

/// Points per wheel notch at `zoom` (1.0 = 100%): more when zoomed in, less when zoomed out, but
/// gently, so zoomed-out overviews don't crawl and close-ups don't jump.
pub fn notch_px(zoom: f32) -> f32 {
    let z = if zoom.is_finite() && zoom > 0.0 { zoom.sqrt().clamp(0.75, 2.0) } else { 1.0 };
    NOTCH_PX * z
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    fn flick(m: &mut Momentum) {
        for _ in 0..6 {
            m.feed(Vec2::new(0.0, -30.0), FRAME);
        }
    }

    #[test]
    fn flick_coasts_then_decays_to_a_stop() {
        let mut m = Momentum::default();
        flick(&mut m);
        m.release();
        let first = m.tick(FRAME);
        assert!(first.y < -10.0, "keeps going the same way after the fingers lift: {first:?}");
        let mut total = first.y;
        let mut last = first.y.abs();
        let mut frames = 0;
        while m.is_active() {
            let d = m.tick(FRAME).y;
            assert!(d.abs() <= last + 1e-3, "slows down");
            last = d.abs();
            total += d;
            frames += 1;
            assert!(frames < 600, "stops within ten seconds");
        }
        // 1800 pt/s with a 1/3 s time constant: a light coast of about 600 pt.
        assert!((-700.0..-400.0).contains(&total), "coasted {total}");
        assert_eq!(m.tick(FRAME), Vec2::ZERO);

        // Frame-rate independent: 144 Hz coasts about as far.
        let mut fast = Momentum::default();
        flick(&mut fast);
        fast.release();
        let mut total144 = 0.0;
        while fast.is_active() {
            total144 += fast.tick(1.0 / 144.0).y;
        }
        assert!((total144 - total).abs() < 30.0, "{total144} vs {total}");
    }

    #[test]
    fn new_input_cancels_and_slow_or_stale_lifts_dont_fling() {
        let mut m = Momentum::default();
        flick(&mut m);
        m.release();
        assert_ne!(m.tick(FRAME), Vec2::ZERO);
        m.cancel();
        assert!(!m.is_active());
        assert_eq!(m.tick(FRAME), Vec2::ZERO);

        // Fingers touching again mid-coast take over: no coasting until they lift again.
        flick(&mut m);
        m.release();
        m.feed(Vec2::new(0.0, -1.0), FRAME);
        assert_eq!(m.tick(FRAME), Vec2::ZERO, "the coast stopped");
        m.release();
        assert!(!m.is_active(), "a slow drag doesn't fling");

        // Resting the fingers before lifting doesn't fling either.
        flick(&mut m);
        for _ in 0..10 {
            m.tick(FRAME);
        }
        m.release();
        assert!(!m.is_active());
    }

    #[test]
    fn hostile_numbers_are_ignored() {
        let mut m = Momentum::default();
        m.feed(Vec2::new(f32::NAN, f32::INFINITY), FRAME);
        m.feed(Vec2::new(0.0, -30.0), f32::NAN);
        assert!(!m.is_active());
        flick(&mut m);
        m.feed(Vec2::new(0.0, f32::NEG_INFINITY), FRAME);
        m.release();
        assert_eq!(m.tick(f32::NAN), Vec2::ZERO);
        assert_eq!(m.tick(f32::INFINITY), Vec2::ZERO);
        let d = m.tick(FRAME);
        assert!(d.is_finite() && d.y < 0.0);
        // A huge delta in a tiny frame is capped.
        let mut m = Momentum::default();
        m.feed(Vec2::new(0.0, 1e30), 1e-9);
        m.release();
        assert!(m.tick(FRAME).length() <= MAX_SPEED * FRAME);
        assert!(notch_px(f32::NAN).is_finite() && notch_px(-1.0) == NOTCH_PX);
    }
}

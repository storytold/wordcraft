//! The caret moves and blinks the way Word's does on Windows, instead of jumping and switching
//! on and off.
//!
//! Measured from Word for Windows (version 16.0.20326, at 100% zoom on a 144 Hz display) as a
//! user sees it: a synthetic document, Word's window captured frame by frame while keys, typing
//! and clicks moved the caret. Nothing was read from the program. Word for Mac was not measured.
//!
//! - **A key or typing moves the caret:** it glides from where it was to its new place in
//!   [`GLIDE_SECS`], fast at first and settling ([`ease`]). The time is the same for one
//!   character and for a whole line.
//! - **Another move before it arrives:** it snaps to where it was heading and glides from there.
//! - **A click moves the caret:** no glide; it is drawn at its new place.
//! - **A command from a script or the control channel:** no glide, no fade.
//! - **The blink is a fade:** each time the caret turns on or off it takes [`GLIDE_SECS`] to get
//!   there. A move by a key or a click turns it on the same way, from however visible it was, so
//!   a caret that was off fades in while it glides.
//!
//! What never glides: a caret that moved because the view changed (zoom, a resized window,
//! scrolling, another document), one that wasn't drawn the frame before (a selection, the canvas
//! unfocused), and moves that start or end off screen. Our choice; Word wasn't measured there.
//!
//! With animations off the caret jumps and blinks on and off, as it always did: File ▸ Options ▸
//! Advanced ▸ Animation (the `ui.caretAnimation` command) turns them off, and so do egui's
//! `animation_time` at 0 (the headless test harness, so scripted screenshots never catch a caret
//! in flight) and `WORDCRAFT_CARET_GLIDE_MS=0`. The variable also sets the duration, for looking
//! at the motion slowed down.
//!
//! The glide runs in content space (screen points from the top-left of the scrolled content), so
//! scrolling under a moving caret doesn't bend its path.

use std::hash::{Hash, Hasher};

use egui::{Pos2, Rect, Ui};

use crate::WordApp;
use crate::canvas::Geometry;

/// How long a glide, and each fade of the blink, takes (seconds).
pub const GLIDE_SECS: f64 = 0.10;
/// The caret is on for this long, then off for this long (seconds).
pub const BLINK_SECS: f64 = 0.53;
/// The longest glide `WORDCRAFT_CARET_GLIDE_MS` can ask for.
const MAX_GLIDE_SECS: f64 = 10.0;
/// Moves shorter than this (screen points) aren't worth a glide.
const MIN_MOVE: f32 = 0.5;
/// How far outside the visible canvas a glide may start or end (screen points).
const OFFSCREEN_SLACK: f32 = 48.0;

/// The caret's bar: its two ends.
pub type Bar = [Pos2; 2];

/// What the canvas knows about the caret in one frame.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Where the caret belongs, in content space.
    pub target: Bar,
    /// Fingerprint of what puts the caret there: the document's revision and the selection.
    pub cause: u64,
    /// Fingerprint of how the document is shown: zoom, page arrangement, which document.
    pub view: u64,
    /// The caret is drawn this frame (the canvas has focus and nothing is selected).
    pub shown: bool,
    /// The part of content space on screen.
    pub viewport: Rect,
    /// egui's clock, in seconds.
    pub now: f64,
    /// egui's frame counter: a caret that skipped a frame doesn't glide from its stale place.
    pub frame_nr: u64,
    /// A key was pressed or text arrived this frame.
    pub keys: bool,
    /// A mouse button is down, or was pressed or released this frame.
    pub pointer: bool,
    /// Seconds since the blink was restarted (by the last command or click).
    pub since_blink: f64,
}

/// How to draw the caret this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Where, in the space the target was given in.
    pub bar: Bar,
    /// How strongly: 0 (not at all) to 1.
    pub opacity: f32,
    /// It is gliding or fading in after a move: repaint every frame.
    pub moving: bool,
}

#[derive(Clone, Copy, Debug)]
struct Glide {
    from: Bar,
    to: Bar,
    started: f64,
}

/// The caret turning on after a move, from however visible it was.
#[derive(Clone, Copy, Debug)]
struct FadeIn {
    from: f32,
    started: f64,
}

/// The caret's motion between frames. Time-based, so it looks the same at any frame rate.
#[derive(Clone, Debug)]
pub struct CaretGlide {
    secs: f64,
    /// Where the caret belonged last frame (content space); `None` when it wasn't drawn.
    target: Option<Bar>,
    /// Where and how strongly it was drawn last frame.
    drawn: Option<Bar>,
    opacity: f32,
    glide: Option<Glide>,
    fade: Option<FadeIn>,
    /// A key or a mouse button was pressed last frame: the canvas handles its keys and its mouse
    /// at the end of a frame, so the move they make is seen one frame later.
    keys: bool,
    pointer: bool,
    cause: u64,
    view: u64,
    frame_nr: u64,
}

impl Default for CaretGlide {
    fn default() -> Self {
        CaretGlide::with_secs(glide_secs())
    }
}

impl CaretGlide {
    /// A caret whose glides and fades take `secs` seconds (0 or less: it jumps and blinks hard).
    pub fn with_secs(secs: f64) -> Self {
        let secs = if secs.is_finite() { secs.clamp(0.0, MAX_GLIDE_SECS) } else { GLIDE_SECS };
        CaretGlide {
            secs,
            target: None,
            drawn: None,
            opacity: 0.0,
            glide: None,
            fade: None,
            keys: false,
            pointer: false,
            cause: 0,
            view: 0,
            frame_nr: 0,
        }
    }

    /// Advance to this frame; returns where and how strongly to draw the caret (content space).
    pub fn frame(&mut self, f: Frame) -> Look {
        let continuous = f.frame_nr.wrapping_sub(self.frame_nr) <= 1;
        let was = self.target.filter(|_| continuous);
        let was_opacity = if was.is_some() { self.opacity } else { 0.0 };
        let edited = f.cause != self.cause;
        let same_view = f.view == self.view;
        let by_keys = f.keys || (self.keys && continuous);
        let by_pointer = f.pointer || (self.pointer && continuous);
        self.cause = f.cause;
        self.view = f.view;
        self.frame_nr = f.frame_nr;
        self.keys = f.keys;
        self.pointer = f.pointer;
        let sane = is_finite(f.target) && f.now.is_finite();
        if !(f.shown && sane) {
            self.reset();
            return Look { bar: f.target, opacity: 0.0, moving: false };
        }
        if self.secs <= 0.0 {
            // Animations off: in place, and on or off.
            self.glide = None;
            self.fade = None;
            let opacity = if blink(f.since_blink, 0.0) > 0.5 { 1.0 } else { 0.0 };
            return self.draw(f.target, f.target, opacity);
        }
        if !same_view {
            self.glide = None;
        }
        // Where the caret was heading: its place last frame, or the end of the glide under way.
        let heading = self.glide.map(|g| g.to).or(was);
        let moved = heading.is_none_or(|h| distance(h, f.target) >= MIN_MOVE);
        if edited && moved && (by_keys || by_pointer) {
            // A key or a click turns the caret on, from however visible it was.
            self.fade = Some(FadeIn { from: was_opacity, started: f.now });
            self.glide = match heading {
                // Keys glide, from where the caret was heading (not from where it has got to).
                Some(from) if by_keys && !by_pointer && same_view => {
                    let seen = f.viewport.expand(OFFSCREEN_SLACK);
                    let on_screen = |b: Bar| seen.intersects(Rect::from_two_pos(b[0], b[1]));
                    (on_screen(from) && on_screen(f.target)).then_some(Glide { from, to: f.target, started: f.now })
                }
                _ => None,
            };
        } else if edited && moved {
            // A script moved it: it is simply there.
            self.glide = None;
            self.fade = None;
        }
        // The text shifted under a glide (pixel snapping while scrolling): carry the glide along.
        if let Some(g) = &mut self.glide
            && g.to != f.target
        {
            let d = f.target[0] - g.to[0];
            g.from = [g.from[0] + d, g.from[1] + d];
            g.to = f.target;
        }
        let bar = match self.glide {
            Some(g) => match progress(f.now, g.started, self.secs) {
                Some(s) => lerp(g.from, g.to, ease(s)),
                None => {
                    self.glide = None;
                    f.target
                }
            },
            None => f.target,
        };
        let opacity = match self.fade {
            Some(fade) => match progress(f.now, fade.started, self.secs) {
                Some(s) => fade.from + (1.0 - fade.from) * s,
                None => {
                    self.fade = None;
                    1.0
                }
            },
            // A gliding caret stays lit; a resting one blinks.
            None if self.glide.is_some() => 1.0,
            None => blink(f.since_blink, self.secs),
        };
        self.draw(f.target, bar, opacity)
    }

    fn draw(&mut self, target: Bar, bar: Bar, opacity: f32) -> Look {
        let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
        self.target = Some(target);
        self.drawn = Some(bar);
        self.opacity = opacity;
        Look { bar, opacity, moving: self.glide.is_some() || self.fade.is_some() }
    }

    /// The caret is on its way somewhere.
    pub fn is_gliding(&self) -> bool {
        self.glide.is_some()
    }

    /// Where the caret was drawn last frame (content space), if it was drawn.
    pub fn drawn(&self) -> Option<Bar> {
        self.drawn
    }

    /// How strongly the caret was drawn last frame (0 when it wasn't).
    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    /// Forget the caret: it appears wherever it belongs next frame.
    pub fn reset(&mut self) {
        self.glide = None;
        self.fade = None;
        self.target = None;
        self.drawn = None;
        self.opacity = 0.0;
    }
}

/// How far along an animation that started at `started` and lasts `secs` is at `now`: 0 to 1, or
/// `None` once it is over (or the clock went backwards).
fn progress(now: f64, started: f64, secs: f64) -> Option<f32> {
    let s = (now - started) / secs;
    (0.0..1.0).contains(&s).then_some(s as f32)
}

/// Word's curve for the glide, `cubic-bezier(0, 0, 0, 1)`: more than a third of the way after
/// 7% of the time, nine tenths after half of it. For these control points the curve has a closed
/// form: time is `u³` and distance `3u² − 2u³`.
pub fn ease(s: f32) -> f32 {
    let s = if s.is_finite() { s.clamp(0.0, 1.0) } else { 1.0 };
    3.0 * s.powf(2.0 / 3.0) - 2.0 * s
}

/// A resting caret's opacity `since` seconds after its blink was restarted: on for
/// [`BLINK_SECS`], off for as long, and so on, taking `fade` seconds over each change (the first
/// turn-on isn't faded here: the move that restarted the blink does that).
pub fn blink(since: f64, fade: f64) -> f32 {
    if !since.is_finite() || since < BLINK_SECS {
        return 1.0;
    }
    let phase = since % (2.0 * BLINK_SECS);
    let (into, turning_on) = if phase < BLINK_SECS { (phase, true) } else { (phase - BLINK_SECS, false) };
    let done = if fade > 0.0 { (into / fade).clamp(0.0, 1.0) as f32 } else { 1.0 };
    if turning_on { done } else { 1.0 - done }
}

/// The blink is part-way through a fade: keep repainting.
fn blink_fading(since: f64, fade: f64) -> bool {
    since.is_finite() && since >= BLINK_SECS && (since % BLINK_SECS) < fade
}

fn lerp(a: Bar, b: Bar, t: f32) -> Bar {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// How far the bar's farthest end moves.
fn distance(a: Bar, b: Bar) -> f32 {
    (b[0] - a[0]).length().max((b[1] - a[1]).length())
}

fn is_finite(b: Bar) -> bool {
    b[0].x.is_finite() && b[0].y.is_finite() && b[1].x.is_finite() && b[1].y.is_finite()
}

/// The glide's duration: [`GLIDE_SECS`], or `WORDCRAFT_CARET_GLIDE_MS` when set (tests ignore it).
fn glide_secs() -> f64 {
    if cfg!(test) {
        return GLIDE_SECS;
    }
    static SECS: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *SECS.get_or_init(|| parse_ms(std::env::var("WORDCRAFT_CARET_GLIDE_MS").ok().as_deref()))
}

fn parse_ms(value: Option<&str>) -> f64 {
    match value.and_then(|s| s.trim().parse::<f64>().ok()) {
        Some(ms) if ms.is_finite() => (ms / 1000.0).clamp(0.0, MAX_GLIDE_SECS),
        _ => GLIDE_SECS,
    }
}

/// Stable id for the caret glide's temporary animation state.
fn state_id() -> egui::Id {
    egui::Id::unique("caret_glide")
}

/// What puts the caret where it is: the document's revision and the caret's position in it.
fn cause_key(app: &WordApp) -> u64 {
    let s = &app.session;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (s.rev(), &s.sel.focus).hash(&mut h);
    if let Some(m) = &s.math {
        format!("{:?}", m.pos).hash(&mut h);
    }
    h.finish()
}

/// How the document is shown: which document, the zoom and where the pages sit.
fn view_key(app: &WordApp, geo: &Geometry) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (app.session.document_id(), geo.scale.to_bits(), geo.cols, geo.size.x.to_bits()).hash(&mut h);
    geo.rects.first().map(|r| (r.min.x.to_bits(), r.min.y.to_bits())).hash(&mut h);
    h.finish()
}

/// The canvas's hook: `target` is where the caret belongs on screen this frame, `shown` whether
/// it is drawn at all, `since_ms` how long ago its blink was restarted and `on` what the plain
/// on-and-off blink says. Returns where and how strongly to draw it. `content_origin` is the scrolled
/// content's top-left on screen.
pub fn look(app: &WordApp, ui: &Ui, content_origin: Pos2, geo: &Geometry, target: Bar, shown: bool, since_ms: f64, on: bool) -> Look {
    // Turned off in Options, Animation effects off (the setting, or the system's under "Use system
    // setting"), or egui's own switch for "no animations": the caret jumps and blinks on and off.
    if !app.ui.caret_animation || !app.animations_on() || ui.style().animation_time <= 0.0 || glide_secs() <= 0.0 {
        ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<CaretGlide>(state_id()).reset());
        return Look { bar: target, opacity: if on { 1.0 } else { 0.0 }, moving: false };
    }
    let off = content_origin.to_vec2();
    let since = since_ms / 1000.0;
    let (keys, pointer, now) = ui.input(|i| {
        let typed = |e: &egui::Event| {
            matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Ime(_))
        };
        (i.events.iter().any(typed), i.pointer.any_down() || i.pointer.any_pressed() || i.pointer.any_released(), i.time)
    });
    let frame = Frame {
        target: [target[0] - off, target[1] - off],
        cause: cause_key(app),
        view: view_key(app, geo),
        shown,
        viewport: ui.clip_rect().translate(-off),
        now,
        frame_nr: ui.ctx().cumulative_frame_nr(),
        keys,
        pointer,
        since_blink: since,
    };
    let (mut look, gliding, secs) = ui.ctx().data_mut(|d| {
        let state = d.get_temp_mut_or_default::<CaretGlide>(state_id());
        (state.frame(frame), state.is_gliding(), state.secs)
    });
    // At rest the caller's own pixel-snapped bar is drawn, untouched.
    look.bar = if gliding { [look.bar[0] + off, look.bar[1] + off] } else { target };
    if look.moving {
        ui.ctx().request_repaint();
    } else if shown && blink_fading(since, secs) {
        // The blink's fade needs a handful of frames, not every one.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    look
}

/// The caret's glide state this frame (for tests and tools).
pub fn state(ctx: &egui::Context) -> CaretGlide {
    ctx.data(|d| d.get_temp::<CaretGlide>(state_id())).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};
    use serde_json::json;

    use super::*;

    const DT: f64 = 1.0 / 60.0;

    fn bar(x: f32, y: f32) -> Bar {
        [pos2(x, y), pos2(x, y + 16.0)]
    }

    /// A caret driven frame by frame, with the bookkeeping a canvas would do.
    struct Rig {
        glide: CaretGlide,
        f: Frame,
        /// When the blink was last restarted.
        restarted: f64,
    }

    impl Rig {
        fn new(at: Bar) -> Self {
            let f = Frame {
                target: at,
                cause: 1,
                view: 1,
                shown: true,
                viewport: Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0)),
                now: 100.0,
                frame_nr: 10,
                keys: false,
                pointer: false,
                since_blink: 0.0,
            };
            let mut rig = Rig { glide: CaretGlide::with_secs(GLIDE_SECS), f, restarted: 100.0 };
            assert_eq!(rig.glide.frame(rig.f), Look { bar: at, opacity: 1.0, moving: false }, "the first frame draws the caret in place");
            rig
        }

        /// The next frame, `dt` seconds later. Input lasts one frame.
        fn step(&mut self, dt: f64) -> Look {
            self.f.now += dt;
            self.f.frame_nr += 1;
            self.f.since_blink = self.f.now - self.restarted;
            let look = self.glide.frame(self.f);
            self.f.keys = false;
            self.f.pointer = false;
            look
        }

        /// A key (arrow, typing) moves the caret: the command restarts the blink.
        fn key_to(&mut self, to: Bar) {
            self.f.target = to;
            self.f.cause += 1;
            self.f.keys = true;
            self.restarted = self.f.now + DT;
        }

        /// A script or the control channel moves the caret.
        fn script_to(&mut self, to: Bar) {
            self.f.target = to;
            self.f.cause += 1;
            self.restarted = self.f.now + DT;
        }

        /// A click moves the caret. The canvas handles the mouse at the end of its frame, so the
        /// button is seen a frame before the caret moves.
        fn click_to(&mut self, to: Bar) {
            self.f.pointer = true;
            self.step(DT);
            self.f.target = to;
            self.f.cause += 1;
            self.restarted = self.f.now + DT;
        }

        /// Let `secs` pass at rest.
        fn rest(&mut self, secs: f64) -> Look {
            let mut look = self.step(DT);
            for _ in 1..(secs / DT).round() as usize {
                look = self.step(DT);
            }
            look
        }
    }

    /// The curve is Word's: these are the caret's measured places, frame by frame at 144 Hz, as
    /// fractions of the way (the same for moves of 6 px and of 372 px).
    #[test]
    fn the_glide_follows_words_measured_curve() {
        let measured = [0.369, 0.529, 0.640, 0.724, 0.790, 0.843, 0.885, 0.918, 0.945, 0.965, 0.980, 0.991, 0.997, 1.0];
        for (k, want) in measured.iter().enumerate() {
            let s = (k + 1) as f32 * (1.0 / 144.0) / GLIDE_SECS as f32;
            assert!((ease(s) - want).abs() < 0.004, "frame {}: {} vs measured {want}", k + 1, ease(s));
        }
        assert_eq!((ease(0.0), ease(1.0)), (0.0, 1.0));
        assert!(ease(0.5) > 0.88, "nine tenths of the way in half the time");
        assert_eq!((ease(-3.0), ease(7.0), ease(f32::NAN)), (0.0, 1.0, 1.0));
        let mut last = 0.0;
        for i in 0..=1000 {
            let p = ease(i as f32 / 1000.0);
            assert!(p >= last && p <= 1.0, "never backwards, never past the end");
            last = p;
        }
    }

    #[test]
    fn a_key_move_glides_and_lands_exactly() {
        let mut r = Rig::new(bar(100.0, 50.0));
        r.key_to(bar(200.0, 90.0));
        let first = r.step(DT);
        assert_eq!(first.bar, bar(100.0, 50.0), "the glide starts from where the caret was");
        assert!(first.moving && r.glide.is_gliding());
        let mut last = 100.0;
        let mut frames = 0;
        while r.glide.is_gliding() {
            let look = r.step(DT);
            let b = look.bar;
            assert!(b[0].x >= last && b[0].x <= 200.0, "moves toward the target without overshooting: {b:?}");
            assert!((b[1].y - b[0].y - 16.0).abs() < 1e-3, "the bar keeps its height: {b:?}");
            assert_eq!(look.opacity, 1.0, "a caret that was lit stays lit on the way");
            last = b[0].x;
            frames += 1;
            assert!(frames < 60);
        }
        assert_eq!(r.glide.drawn(), Some(bar(200.0, 90.0)));
        // 0.1 s at 60 Hz.
        assert!((5..=7).contains(&frames), "{frames} frames");
        assert_eq!(r.step(DT), Look { bar: bar(200.0, 90.0), opacity: 1.0, moving: false }, "then it rests");
    }

    #[test]
    fn the_glide_is_the_same_at_any_frame_rate() {
        let at = |hz: f64| {
            let mut r = Rig::new(bar(0.0, 0.0));
            r.key_to(bar(300.0, 0.0));
            r.step(1.0 / hz);
            let mut b = bar(0.0, 0.0);
            // 0.05 s in: three frames at 60 Hz, twelve at 240 Hz.
            for _ in 0..(hz / 20.0).round() as usize {
                b = r.step(1.0 / hz).bar;
            }
            b[0].x
        };
        let expected = 300.0 * ease((0.05 / GLIDE_SECS) as f32);
        assert!((at(60.0) - expected).abs() < 0.5, "{} vs {expected}", at(60.0));
        assert!((at(240.0) - expected).abs() < 0.5, "{} vs {expected}", at(240.0));
    }

    /// Word: a second key before the caret arrives makes it snap to where it was heading and set
    /// off from there (held arrow keys, quick typing).
    #[test]
    fn a_new_key_move_sets_off_from_where_the_caret_was_heading() {
        let mut r = Rig::new(bar(0.0, 0.0));
        r.key_to(bar(100.0, 0.0));
        r.step(DT);
        let mid = r.step(DT * 2.0).bar;
        assert!(mid[0].x > 0.0 && mid[0].x < 100.0);
        r.key_to(bar(110.0, 0.0));
        assert_eq!(r.step(DT).bar, bar(100.0, 0.0), "it snaps to the first move's end");
        let next = r.step(DT).bar;
        assert!(next[0].x > 100.0 && next[0].x < 110.0);
        r.rest(0.2);
        assert_eq!(r.glide.drawn(), Some(bar(110.0, 0.0)));
        assert!(!r.glide.is_gliding());
        // Something else changed the document but left the caret alone: nothing to do.
        r.f.cause += 1;
        r.f.keys = true;
        assert_eq!(r.step(DT).bar, bar(110.0, 0.0));
        assert!(!r.glide.is_gliding());
        // The canvas reads its keys at the end of a frame: the move shows up one frame after the
        // key, and still glides. Two frames after, it is somebody else's move.
        r.rest(0.2);
        r.f.keys = true;
        r.step(DT);
        r.f.target = bar(160.0, 0.0);
        r.f.cause += 1;
        assert_eq!(r.step(DT).bar, bar(110.0, 0.0));
        assert!(r.glide.is_gliding());
        r.rest(0.2);
        r.f.keys = true;
        r.step(DT);
        r.step(DT);
        r.f.target = bar(210.0, 0.0);
        r.f.cause += 1;
        assert_eq!(r.step(DT).bar, bar(210.0, 0.0));
        assert!(!r.glide.is_gliding());
    }

    /// Word: a click puts the caret where it was clicked, a script's move likewise; only keys glide.
    #[test]
    fn clicks_and_scripts_do_not_glide() {
        let mut r = Rig::new(bar(100.0, 100.0));
        r.click_to(bar(400.0, 300.0));
        let look = r.step(DT);
        assert_eq!(look.bar, bar(400.0, 300.0));
        assert!(!r.glide.is_gliding());
        assert_eq!(look.opacity, 1.0, "a lit caret stays lit");
        r.rest(0.2);
        r.script_to(bar(50.0, 50.0));
        assert_eq!(r.step(DT), Look { bar: bar(50.0, 50.0), opacity: 1.0, moving: false });
        // A script's move during a glide ends the glide.
        r.key_to(bar(250.0, 50.0));
        r.step(DT);
        r.step(DT);
        assert!(r.glide.is_gliding());
        r.script_to(bar(20.0, 20.0));
        assert_eq!(r.step(DT), Look { bar: bar(20.0, 20.0), opacity: 1.0, moving: false });
        // A click during a glide too.
        r.key_to(bar(250.0, 20.0));
        r.step(DT);
        r.click_to(bar(300.0, 300.0));
        assert_eq!(r.step(DT).bar, bar(300.0, 300.0));
        assert!(!r.glide.is_gliding());
    }

    /// Word: the blink is a fade, and a move turns the caret on from however visible it was.
    #[test]
    fn the_blink_fades_and_a_move_turns_the_caret_on() {
        // On for 0.53 s, a 0.1 s fade out, off, a 0.1 s fade in.
        assert_eq!((blink(0.0, 0.1), blink(0.3, 0.1), blink(0.529, 0.1)), (1.0, 1.0, 1.0));
        assert!((blink(0.58, 0.1) - 0.5).abs() < 0.01);
        assert_eq!((blink(0.64, 0.1), blink(1.0, 0.1)), (0.0, 0.0));
        assert!((blink(1.06 + 0.025, 0.1) - 0.25).abs() < 0.01);
        assert_eq!(blink(1.3, 0.1), 1.0);
        assert!((blink(1.59 + 0.075, 0.1) - 0.25).abs() < 0.01);
        assert!(blink_fading(0.55, 0.1) && blink_fading(1.1, 0.1) && !blink_fading(0.3, 0.1) && !blink_fading(0.8, 0.1) && !blink_fading(0.05, 0.1));
        // With no fade it is the plain on-and-off blink.
        assert_eq!((blink(0.52, 0.0), blink(0.54, 0.0), blink(1.05, 0.0), blink(1.07, 0.0)), (1.0, 0.0, 0.0, 1.0));
        assert_eq!((blink(f64::NAN, 0.1), blink(-5.0, 0.1), blink(f64::INFINITY, 0.1)), (1.0, 1.0, 1.0));

        // At rest the caret fades out and back in.
        let mut r = Rig::new(bar(100.0, 100.0));
        assert_eq!(r.rest(0.4).opacity, 1.0);
        let fading = r.rest(0.18).opacity;
        assert!(fading > 0.2 && fading < 0.8, "half-way through the fade out: {fading}");
        assert_eq!(r.rest(0.2).opacity, 0.0);
        // A key while it is off: it fades in over the glide, evenly.
        r.key_to(bar(200.0, 100.0));
        let mut seen = vec![r.step(DT).opacity];
        while r.glide.is_gliding() {
            seen.push(r.step(DT).opacity);
        }
        assert_eq!((seen[0], seen[seen.len() - 1]), (0.0, 1.0));
        assert!(seen.windows(2).all(|w| w[1] >= w[0]) && seen[1] > 0.1 && seen[2] > seen[1], "{seen:?}");
        assert!((seen[3] - 0.5).abs() < 0.02, "half lit half-way: {seen:?}");
        assert_eq!(r.step(DT).opacity, 1.0);
        // A click while it is off fades in too, in place.
        r.rest(0.8);
        assert_eq!(r.glide.opacity(), 0.0);
        r.click_to(bar(300.0, 200.0));
        let look = r.step(DT);
        assert_eq!((look.bar, look.opacity, look.moving), (bar(300.0, 200.0), 0.0, true));
        let look = r.rest(0.05);
        assert!(look.opacity > 0.3 && look.opacity < 0.7 && look.bar == bar(300.0, 200.0), "{look:?}");
        assert_eq!(r.rest(0.1).opacity, 1.0);
        // A script's move while it is off: lit at once.
        r.rest(0.7);
        assert_eq!(r.glide.opacity(), 0.0);
        r.script_to(bar(10.0, 10.0));
        assert_eq!(r.step(DT), Look { bar: bar(10.0, 10.0), opacity: 1.0, moving: false });
        // A key while it is half faded: on from there.
        r.rest(0.56);
        let half = r.glide.opacity();
        assert!(half > 0.1 && half < 0.9, "{half}");
        r.key_to(bar(60.0, 10.0));
        assert!((r.step(DT).opacity - half).abs() < 1e-6);
        assert!(r.step(DT).opacity > half);
    }

    #[test]
    fn view_changes_and_scrolling_never_glide() {
        // Zoom or a resized window: the caret stays glued to its text.
        let mut r = Rig::new(bar(100.0, 100.0));
        r.f.target = bar(180.0, 150.0);
        r.f.view += 1;
        assert_eq!(r.step(DT).bar, bar(180.0, 150.0));
        assert!(!r.glide.is_gliding());
        // …even when a key press in the same frame also moved the caret.
        r.key_to(bar(40.0, 40.0));
        r.f.view += 1;
        assert_eq!(r.step(DT).bar, bar(40.0, 40.0));
        assert!(!r.glide.is_gliding());
        r.rest(0.2);
        // The target shifting with nothing edited (pixel snapping while scrolling) is followed.
        r.f.target = bar(40.4, 40.0);
        assert_eq!(r.step(DT).bar, bar(40.4, 40.0));
        assert!(!r.glide.is_gliding());
        // The same shift under a glide moves the whole glide; it doesn't restart it.
        r.key_to(bar(140.0, 40.0));
        r.step(DT);
        let before = r.step(DT).bar;
        r.f.target = bar(140.0, 40.5);
        let after = r.step(DT).bar;
        assert!(after[0].x > before[0].x && (after[0].y - 40.5).abs() < 1e-3, "{after:?}");
        r.rest(0.2);
        assert_eq!(r.glide.drawn(), Some(bar(140.0, 40.5)));
        // A view change during a glide ends it.
        r.key_to(bar(340.0, 40.5));
        r.step(DT);
        r.step(DT);
        r.f.view += 1;
        assert_eq!(r.step(DT).bar, bar(340.0, 40.5));
        assert!(!r.glide.is_gliding());
    }

    #[test]
    fn a_hidden_stale_or_off_screen_caret_appears_in_place() {
        // Hidden (a selection, or the canvas lost focus), then back somewhere else.
        let mut r = Rig::new(bar(100.0, 100.0));
        r.f.shown = false;
        r.key_to(bar(300.0, 100.0));
        assert_eq!(r.step(DT).opacity, 0.0);
        assert_eq!(r.glide.drawn(), None);
        r.f.shown = true;
        r.key_to(bar(500.0, 100.0));
        let look = r.step(DT);
        assert_eq!(look.bar, bar(500.0, 100.0));
        assert!(!r.glide.is_gliding() && look.moving, "it fades in where it is");
        r.rest(0.2);
        // The canvas skipped frames (Backstage open): no glide from the stale place.
        r.f.frame_nr += 30;
        r.key_to(bar(200.0, 100.0));
        assert_eq!(r.step(DT).bar, bar(200.0, 100.0));
        assert!(!r.glide.is_gliding());
        r.rest(0.2);
        // A jump to somewhere off screen (Ctrl+End), and back from there.
        r.key_to(bar(200.0, 5000.0));
        assert_eq!(r.step(DT).bar, bar(200.0, 5000.0));
        assert!(!r.glide.is_gliding());
        r.key_to(bar(200.0, 100.0));
        assert_eq!(r.step(DT).bar, bar(200.0, 100.0));
        assert!(!r.glide.is_gliding());
        r.rest(0.2);
        // Just past the edge (the next line while the view scrolls to it) still glides.
        r.key_to(bar(200.0, 810.0));
        r.step(DT);
        assert!(r.glide.is_gliding());
    }

    #[test]
    fn turned_off_or_hostile_numbers_never_glide_or_panic() {
        // Off: it jumps, and blinks hard.
        let mut off = Rig::new(bar(0.0, 0.0));
        off.glide = CaretGlide::with_secs(0.0);
        off.step(DT);
        off.key_to(bar(90.0, 0.0));
        assert_eq!(off.step(DT), Look { bar: bar(90.0, 0.0), opacity: 1.0, moving: false });
        assert_eq!(off.rest(0.5).opacity, 1.0);
        assert_eq!(off.rest(0.06).opacity, 0.0);
        assert_eq!(off.rest(0.5).opacity, 1.0);

        assert_eq!(CaretGlide::with_secs(f64::NAN).secs, GLIDE_SECS);
        assert_eq!(CaretGlide::with_secs(-3.0).secs, 0.0);
        assert_eq!(CaretGlide::with_secs(f64::INFINITY).secs, GLIDE_SECS);
        assert_eq!(parse_ms(None), GLIDE_SECS);
        assert_eq!(parse_ms(Some("junk")), GLIDE_SECS);
        assert_eq!(parse_ms(Some("NaN")), GLIDE_SECS);
        assert_eq!(parse_ms(Some(" 0 ")), 0.0);
        assert_eq!(parse_ms(Some("-5")), 0.0);
        assert_eq!(parse_ms(Some("250")), 0.25);
        assert_eq!(parse_ms(Some("1e300")), MAX_GLIDE_SECS);

        let mut r = Rig::new(bar(10.0, 10.0));
        let junk = [pos2(f32::NAN, f32::INFINITY), pos2(f32::NEG_INFINITY, 0.0)];
        r.key_to(junk);
        assert_eq!(r.step(DT).opacity, 0.0);
        assert!(!r.glide.is_gliding() && r.glide.drawn().is_none());
        r.key_to(bar(50.0, 10.0));
        assert_eq!(r.step(DT).bar, bar(50.0, 10.0), "back from junk: in place");
        r.rest(0.2);
        // The clock stops, jumps back or goes bad mid-glide.
        r.key_to(bar(150.0, 10.0));
        r.step(DT);
        assert!(is_finite(r.step(0.0).bar));
        assert_eq!(r.step(-50.0).bar, bar(150.0, 10.0));
        r.key_to(bar(250.0, 10.0));
        r.step(DT);
        assert_eq!(r.step(f64::NAN).opacity, 0.0, "a bad clock: nothing is drawn that frame");
        r.f.now = 200.0;
        r.key_to(bar(350.0, 10.0));
        r.step(DT);
        r.step(f64::INFINITY);
        // A blink clock gone bad leaves the caret lit.
        let mut r = Rig::new(bar(0.0, 0.0));
        r.restarted = f64::NAN;
        let look = r.step(DT);
        assert!(look.opacity == 1.0 && is_finite(look.bar));
        r.restarted = f64::INFINITY;
        assert_eq!(r.step(DT).opacity, 1.0);
        // Coordinates at the edge of f32.
        let mut r = Rig::new(bar(0.0, 0.0));
        r.f.viewport = Rect::EVERYTHING;
        r.key_to(bar(f32::MAX, 0.0));
        r.step(DT);
        for _ in 0..12 {
            let look = r.step(DT);
            assert!((0.0..=1.0).contains(&look.opacity));
        }
        r.glide.reset();
        assert!(r.glide.drawn().is_none() && !r.glide.is_gliding() && r.glide.opacity() == 0.0);
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    /// The whole path: a key in the real app glides the drawn caret across the canvas; a command
    /// run by a script and a change of zoom don't.
    #[test]
    fn the_canvas_caret_glides_after_a_key() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Some text here"})).unwrap();
        // Animation effects off: the caret follows that switch.
        a.run("ui.animations", json!({"value": "off"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(DT as f32).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..3 {
            h.step();
        }
        assert!(state(&h.ctx).drawn().is_none(), "no animations: the glide stands down");
        h.state_mut().run("ui.animations", json!({"value": "on"})).unwrap();
        h.ctx.all_styles_mut(|s| s.animation_time = 0.1);
        for _ in 0..30 {
            h.step();
        }
        let near = |a: Bar, b: Bar| distance(a, b) < 0.01;
        let end = state(&h.ctx).drawn().expect("the focused canvas draws a caret");
        assert!(!state(&h.ctx).is_gliding());
        // Home: the caret glides to the start of the line.
        // (The canvas reads the key at the end of its frame; the caret sets off in the next.)
        h.event(key(egui::Key::Home));
        for _ in 0..3 {
            h.step();
        }
        let mid = state(&h.ctx);
        assert!(mid.is_gliding(), "the caret is on its way");
        let x = mid.drawn().unwrap()[0].x;
        for _ in 0..20 {
            h.step();
        }
        let start = state(&h.ctx).drawn().unwrap();
        assert!(!state(&h.ctx).is_gliding(), "and comes to rest");
        assert_eq!(h.state().session.sel.focus.off, 0, "Home moved the caret");
        assert!(start[0].x + 20.0 < end[0].x, "to the start of the line: {start:?} from {end:?}");
        assert!(x > start[0].x && x < end[0].x, "on the way it was drawn in between: {x}");
        // A command run by a script puts it there at once.
        h.state_mut().run("caret.docEnd", json!({})).unwrap();
        h.step();
        assert!(!state(&h.ctx).is_gliding());
        assert!(near(state(&h.ctx).drawn().unwrap(), end), "back at the end of the text");
        // Zooming moves the caret on screen but doesn't glide it.
        h.state_mut().run("view.zoom", json!({"value": 150})).unwrap();
        for _ in 0..3 {
            h.step();
            assert!(!state(&h.ctx).is_gliding());
        }
        assert!(!near(state(&h.ctx).drawn().unwrap(), end));
        // Turned off in Options: the caret jumps.
        h.state_mut().run("ui.caretAnimation", json!({"value": false})).unwrap();
        h.step();
        assert!(state(&h.ctx).drawn().is_none(), "the animation stands down");
        h.event(key(egui::Key::Home));
        for _ in 0..3 {
            h.step();
            assert!(!state(&h.ctx).is_gliding());
        }
        assert_eq!(h.state().session.sel.focus.off, 0, "Home still moved the caret");
        // And on again.
        h.state_mut().run("ui.caretAnimation", json!({"value": true})).unwrap();
        h.step();
        assert!(state(&h.ctx).drawn().is_some());
    }
}

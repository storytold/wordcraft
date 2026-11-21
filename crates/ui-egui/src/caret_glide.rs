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
//! Advanced ▸ Animation (the `ui.caretAnimation` command) turns them off, and so does any motion
//! mode but the full one (the app's Animation setting and the system's reduce-motion).
//!
//! The glide runs in content space (screen points from the top-left of the scrolled content), so
//! scrolling under a moving caret doesn't bend its path.

use std::hash::{Hash, Hasher};

use craft::behavior::motion::MotionMode;
use egui::{Pos2, Rect};

use crate::WordApp;

/// How long a glide, and each fade of the blink, takes (seconds).
pub const GLIDE_SECS: f64 = 0.10;
/// The caret is on for this long, then off for this long (seconds).
pub const BLINK_SECS: f64 = 0.53;

/// Moves shorter than this (screen points) aren't worth a glide.
const MIN_MOVE: f32 = 0.5;
/// How far outside the visible canvas a glide may start or end (screen points).
const OFFSCREEN_SLACK: f32 = 48.0;

/// The caret's bar: its two ends.
pub type Bar = [Pos2; 2];

/// What the page knows about the caret in one pass.
pub struct LookRequest {
    /// The scrolled content's top-left on screen.
    pub content_origin: Pos2,
    /// Where the caret belongs on screen.
    pub target: Bar,
    /// Whether the caret is drawn at all.
    pub shown: bool,
    /// Milliseconds since the blink was restarted.
    pub since_ms: f64,
    /// What the plain on-and-off blink says.
    pub on: bool,
    /// The pass time, in seconds.
    pub now: f64,
    /// The frame counter: a caret that skipped a frame doesn't glide from its stale place.
    pub frame_nr: u64,
    /// The part of the screen the canvas covers.
    pub viewport: Rect,
    /// The pass took editor input.
    pub keys: bool,
    /// A pointer press is in progress or ended this pass.
    pub pointer: bool,
    /// How much motion the interface shows.
    pub motion: MotionMode,
    /// A fingerprint of the view this pass shows (zoom, scroll, size): a caret that moved with a
    /// changed view doesn't glide.
    pub view: u64,
}

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
    /// The monotonic time (ms) of the last [`CaretGlide::restart`]; 0 until one.
    restarted: f64,
}

impl Default for CaretGlide {
    fn default() -> Self {
        CaretGlide::with_secs(GLIDE_SECS)
    }
}

impl CaretGlide {
    /// A caret whose glides and fades take `secs` seconds (0 or less: it jumps and blinks hard).
    pub fn with_secs(secs: f64) -> Self {
        let secs = if secs.is_finite() { secs.max(0.0) } else { GLIDE_SECS };
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
            restarted: 0.0,
        }
    }

    /// How long a glide and each fade of the blink take, in seconds.
    pub fn secs(&self) -> f64 {
        self.secs
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

    /// Restarts the blink from fully shown at `now_ms` (monotonic milliseconds) after caret
    /// activity. The drawn caret and its glide stay, unlike [`reset`](Self::reset).
    pub fn restart(&mut self, now_ms: f64) {
        self.restarted = now_ms;
        self.fade = None;
        if self.drawn.is_some() {
            self.opacity = 1.0;
        }
    }

    /// Milliseconds from the last [`restart`](Self::restart) to `now_ms` (from creation when none):
    /// the caret's idle time, which the blink and AutoSave's wait read.
    pub fn since_restart_ms(&self, now_ms: f64) -> f64 {
        now_ms - self.restarted
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
pub(crate) fn blink_fading(since: f64, fade: f64) -> bool {
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

/// What puts the caret where it is: the document's revision and the caret's position in it.
// lean: cause fingerprint until the engine emits a change cause
fn cause_key(app: &WordApp) -> u64 {
    let s = &app.session;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (s.rev(), &s.sel.focus).hash(&mut h);
    if let Some(m) = &s.math {
        format!("{:?}", m.pos).hash(&mut h);
    }
    h.finish()
}

/// Steps `glide` for this pass: where and how strongly to draw the caret. With the caret
/// animation off, or any motion mode but the full one, the caret jumps and blinks on and off.
pub fn look(glide: &mut CaretGlide, app: &WordApp, request: LookRequest) -> Look {
    let LookRequest { content_origin, target, shown, since_ms, on, now, frame_nr, viewport, keys, pointer, motion, view } = request;
    if !app.ui.caret_animation || !matches!(motion, MotionMode::Full) {
        glide.reset();
        return Look { bar: target, opacity: if on { 1.0 } else { 0.0 }, moving: false };
    }
    let off = content_origin.to_vec2();
    let frame = Frame {
        target: [target[0] - off, target[1] - off],
        cause: cause_key(app),
        view,
        shown,
        viewport: viewport.translate(-off),
        now,
        frame_nr,
        keys,
        pointer,
        since_blink: since_ms / 1000.0,
    };
    let mut look = glide.frame(frame);
    // At rest the caller's own pixel-snapped bar is drawn, untouched.
    look.bar = if glide.is_gliding() { [look.bar[0] + off, look.bar[1] + off] } else { target };
    look
}

#[cfg(test)]
mod tests;

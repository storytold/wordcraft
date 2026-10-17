//! One-finger gestures on a touch screen (#1503, #2155).
//!
//! Backends turn a finger into ordinary pointer events (`PointerButton`, `PointerMoved`) *and*
//! `Event::Touch`, so without this the canvas cannot tell a swipe from a mouse drag. The page
//! follows the finger 1:1, a tap still places the caret (the canvas does that when the gesture
//! starts), a long press turns into a text selection, and a pinch is left to
//! [`crate::scroll::CanvasScroll::zoom`].
//!
//! A press that starts a selection is not this gesture's to cancel: `TouchDrag::press` only
//! records where it began. The canvas places the caret and, when a long press has latched
//! (`TouchDrag::selecting`), starts the selection.
//!
//! eframe's web runner runs a frame inside `pointerdown`, a browser event that comes before
//! `touchstart`, so on the web the first frame of a touch is indistinguishable from a mouse
//! press. `TouchDrag::arm` remembers such a press and `TouchDrag::adopt` turns it into a
//! finger gesture when the `Touch Start` arrives a frame later; the selection the press started
//! is put back if the finger then pans (`TouchDrag::take_restore`).

use egui::{InputState, Pos2, TouchId, TouchPhase, Vec2};
use wordcraft_engine::Selection;

use crate::scroll::Momentum;

/// How far (screen points) a finger may travel before the press is a pan rather than a tap.
/// A little more forgiving than a mouse's click slop: fingertips are less precise.
const SLOP: f32 = 8.0;

/// A web `pointerdown` press is adopted as a touch when the finger lands this close to it…
const ADOPT_DIST: f32 = 16.0;
/// …and within this many seconds (the next frame is normally 1/60 s).
const ADOPT_TIME: f64 = 0.25;

/// Longest frame time trusted for velocity and decay (a stall shouldn't fling the page).
const MAX_DT: f32 = 0.1;

/// A press that may turn out to be a finger (see [`TouchDrag::arm`]).
#[derive(Clone, Debug)]
struct Candidate {
    pos: Pos2,
    time: f64,
    before: Selection,
}

/// The page's one-finger gesture: pan, tap, long-press selection, or pinch.
#[derive(Clone, Debug, Default)]
pub(crate) struct TouchDrag {
    /// A finger owns the page.
    down: bool,
    /// The finger is dragging the page.
    panning: bool,
    /// The press is a long-press selection; it never pans.
    selecting: bool,
    /// The canvas has already put the caret down and started the selection.
    selection_started: bool,
    /// A pinch is in progress: no pan or selection until every finger lifts.
    pinching: bool,
    /// Where the finger went down.
    origin: Pos2,
    /// Where it was last frame.
    last: Pos2,
    /// The first panning frame feeds the whole travel since the press, so nothing is lost to the
    /// slop; later frames feed their own movement.
    first_pan: bool,
    /// The selection before the press, restored when a pan starts instead of a selection.
    before: Option<Selection>,
    /// Set when a pan starts: the canvas puts this selection back.
    restore: Option<Selection>,
    /// A web press waiting for its `Touch Start`.
    candidate: Option<Candidate>,
    /// Fingers currently on the screen, by touch id. eframe/egui can report `multi_touch` a
    /// frame after the second finger lands, so count the touch events ourselves.
    active: Vec<TouchId>,
    /// Where the finger lifted this frame; the canvas ends the drag it may have started.
    released_at: Option<Pos2>,
    /// The pan's coast after the finger lifts.
    momentum: Momentum,
}

impl TouchDrag {
    /// A finger went down on the page (native backends report the touch and pointer together).
    /// `before` is the selection the press replaced, in case it turns into a pan.
    pub(crate) fn press(&mut self, pos: Pos2, before: Selection) {
        if self.down {
            return;
        }
        self.momentum.cancel();
        self.candidate = None;
        self.pinching = false;
        self.released_at = None;
        self.down = true;
        self.panning = false;
        self.selecting = false;
        self.selection_started = false;
        self.first_pan = false;
        self.origin = pos;
        self.last = pos;
        self.before = Some(before);
        self.restore = None;
    }

    /// A primary press that may be a web `pointerdown` (the browser sends `touchstart` after it).
    /// The normal press still runs; [`Self::adopt`] converts it if a finger lands here next frame.
    pub(crate) fn arm(&mut self, pos: Pos2, now: f64, before: Selection) {
        if self.down || self.pinching {
            return;
        }
        // A new press stops any coast from the last finger.
        self.momentum.cancel();
        self.released_at = None;
        self.candidate = Some(Candidate { pos, time: now, before });
    }

    /// The browser's `Touch Start` arrived. If it belongs to the press armed last frame, the press
    /// was a finger: begin the gesture and tell the canvas to cancel the text drag it started.
    pub(crate) fn adopt(&mut self, pos: Pos2, now: f64) -> bool {
        let Some(candidate) = self.candidate.take() else { return false };
        let age = now - candidate.time;
        if !age.is_finite() || age < 0.0 || age > ADOPT_TIME || !pos.is_finite() || pos.distance(candidate.pos) > ADOPT_DIST {
            return false;
        }
        self.press(pos, candidate.before);
        true
    }

    /// This frame's finger travel (screen points, positive = content moves down/right), or the
    /// coast after the finger lifted. `momentum` false in Read Mode, where columns snap.
    pub(crate) fn advance(&mut self, input: &InputState, dt: f32, long_press: f64, momentum: bool) -> Vec2 {
        self.track_touches(input);
        let multi = input.multi_touch().is_some() || self.active.len() >= 2;
        self.step(input.time, input.any_touches(), multi, input.pointer.latest_pos(), input.pointer.press_start_time(), dt, long_press, momentum)
    }

    /// Count the fingers from the raw touch events. `multi_touch()` is `None` on the frame the
    /// second finger lands, and on a pinch that frame must not place a caret or start a pan.
    fn track_touches(&mut self, input: &InputState) {
        if input.any_touches() {
            // A new finger starts a new gesture; don't report the last one's release to it.
            self.released_at = None;
        }
        if !input.any_touches() {
            self.active.clear();
            return;
        }
        for event in &input.events {
            if let egui::Event::Touch { id, phase, .. } = event {
                match phase {
                    TouchPhase::Start => {
                        // A finger landing stops any coast from the last flick at once.
                        self.momentum.cancel();
                        if !self.active.contains(id) {
                            self.active.push(*id);
                        }
                    }
                    TouchPhase::End | TouchPhase::Cancel => self.active.retain(|t| t != id),
                    TouchPhase::Move => {}
                }
            }
        }
    }

    /// [`Self::advance`] with the frame's input unpacked, for tests.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn step(
        &mut self,
        now: f64,
        touches: bool,
        multi: bool,
        pointer: Option<Pos2>,
        press_start: Option<f64>,
        dt: f32,
        long_press: f64,
        momentum: bool,
    ) -> Vec2 {
        let dt = if dt.is_finite() { dt.clamp(0.0, MAX_DT) } else { 0.0 };
        // A web press whose finger never arrived was a mouse click after all.
        if let Some(candidate) = &self.candidate
            && now - candidate.time > ADOPT_TIME
        {
            self.candidate = None;
        }
        if self.pinching {
            if !touches {
                self.pinching = false;
            }
            return Vec2::ZERO;
        }
        if !touches {
            if self.down {
                if self.panning && momentum {
                    self.momentum.release();
                }
                self.released_at = Some(self.last);
                self.reset();
            }
            return if momentum { self.momentum.tick(dt) } else { Vec2::ZERO };
        }
        if multi {
            // A second finger: the pinch owns the gesture. Don't let the first finger place a
            // caret, keep selecting or keep scrolling underneath it — and put back the
            // selection the first finger's press replaced.
            let before = self.before.take().or_else(|| self.restore.take());
            self.pinching = true;
            self.reset();
            self.restore = before;
            self.momentum.cancel();
            return Vec2::ZERO;
        }
        if !self.down {
            return Vec2::ZERO;
        }
        let Some(pos) = pointer else { return Vec2::ZERO };
        if !self.panning && !self.selecting {
            let travel = self.origin.distance(pos);
            if travel > SLOP {
                self.panning = true;
                self.first_pan = true;
                // A pan must not throw away a selection the press replaced.
                self.restore = self.before.take();
            } else if press_start.is_some_and(|start| now - start >= long_press) {
                // Held still: Word's model, selection instead of panning.
                self.selecting = true;
            }
        }
        let mut delta = Vec2::ZERO;
        if self.panning {
            let step = if self.first_pan { pos - self.origin } else { pos - self.last };
            self.first_pan = false;
            self.last = pos;
            if step.is_finite() && step != Vec2::ZERO {
                if momentum {
                    self.momentum.feed(step, dt);
                }
                delta += step;
            }
        } else {
            self.last = pos;
        }
        delta
    }

    /// The finger lifted this frame: the canvas ends a selection drag it started. The
    /// position is for a double-tap whose release frame also carried `PointerGone`.
    pub(crate) fn take_release(&mut self) -> Option<Pos2> {
        self.released_at.take()
    }

    /// The selection to put back when a pan starts, once.
    pub(crate) fn take_restore(&mut self) -> Option<Selection> {
        self.restore.take()
    }

    pub(crate) fn down(&self) -> bool {
        self.down
    }

    pub(crate) fn panning(&self) -> bool {
        self.panning
    }

    pub(crate) fn pinching(&self) -> bool {
        self.pinching
    }

    pub(crate) fn selecting(&self) -> bool {
        self.selecting
    }

    pub(crate) fn selection_started(&self) -> bool {
        self.selection_started
    }

    pub(crate) fn mark_selection_started(&mut self) {
        self.selection_started = true;
    }

    /// The scroll ran into the end of the document: stop the coast, keep the finger's gesture.
    pub(crate) fn cancel_momentum(&mut self) {
        self.momentum.cancel();
    }

    /// Keep repainting while the page coasts after a flick.
    pub(crate) fn is_animating(&self) -> bool {
        self.momentum.is_active()
    }

    /// Forget the gesture (a mode change, a dialog opening).
    pub(crate) fn cancel(&mut self) {
        self.reset();
        self.momentum.cancel();
        self.candidate = None;
        self.pinching = false;
        self.released_at = None;
        self.active.clear();
    }

    fn reset(&mut self) {
        self.down = false;
        self.panning = false;
        self.selecting = false;
        self.selection_started = false;
        self.first_pan = false;
        self.before = None;
        self.restore = None;
        self.last = Pos2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_engine::Selection;

    const FRAME: f32 = 1.0 / 60.0;
    const LONG: f64 = 0.6;

    fn selection() -> Selection {
        Selection::caret(wordcraft_doc::Pos::body(0, 0))
    }

    fn down(t: &mut TouchDrag, at: Pos2) {
        t.press(at, selection());
    }

    /// A still frame with one finger down at `at`.
    fn still(t: &mut TouchDrag, at: Pos2, now: f64) -> Vec2 {
        t.step(now, true, false, Some(at), Some(0.0), FRAME, LONG, true)
    }

    #[test]
    fn a_drag_pans_one_to_one_and_coasts_after_a_flick() {
        let mut t = TouchDrag::default();
        down(&mut t, Pos2::ZERO);
        assert!(t.down() && !t.panning());
        let mut total = 0.0;
        for k in 1..=6 {
            let at = Pos2::new(0.0, -30.0 * k as f32);
            let d = still(&mut t, at, f64::from(k) / 60.0);
            assert!(d.y <= 0.0, "the page follows the finger: {d:?}");
            total += d.y;
        }
        assert!(t.panning());
        assert!((-180.0..-150.0).contains(&total), "1:1 travel: {total}");
        assert!(t.take_restore().is_some(), "a pan puts the old selection back");

        // Lift: the flick coasts on, then stops.
        let first = t.step(7.0 / 60.0, false, false, None, None, FRAME, LONG, true);
        assert!(first.y < 0.0, "coasts: {first:?}");
        let mut coast = first.y;
        for k in 8..600 {
            if !t.is_animating() {
                break;
            }
            coast += t.step(f64::from(k) / 60.0, false, false, None, None, FRAME, LONG, true).y;
        }
        assert!(coast < -50.0, "coasted {coast}");
        assert!(!t.is_animating());
        assert!(!t.down() && !t.panning());
    }

    #[test]
    fn a_tap_does_not_pan_or_coast() {
        let mut t = TouchDrag::default();
        down(&mut t, Pos2::new(10.0, 10.0));
        assert_eq!(still(&mut t, Pos2::new(12.0, 13.0), FRAME as f64), Vec2::ZERO);
        assert!(!t.panning());
        assert_eq!(t.step(2.0 * f64::from(FRAME), false, false, None, None, FRAME, LONG, true), Vec2::ZERO);
        assert!(!t.is_animating());
    }

    #[test]
    fn a_long_press_selects_and_never_pans() {
        let mut t = TouchDrag::default();
        down(&mut t, Pos2::new(40.0, 40.0));
        for k in 1..=40 {
            let d = still(&mut t, Pos2::new(40.0, 40.0), k as f64 * f64::from(FRAME));
            assert_eq!(d, Vec2::ZERO);
            assert!(!t.panning());
        }
        assert!(t.selecting(), "held past the long-press delay");
        assert!(t.take_restore().is_none());
        // Moving now extends the selection (the canvas does that) but never pans.
        let d = still(&mut t, Pos2::new(40.0, -100.0), 1.0);
        assert_eq!(d, Vec2::ZERO);
        assert!(t.selecting() && !t.panning());
    }

    #[test]
    fn a_pinch_stops_the_pan_until_every_finger_lifts() {
        let mut t = TouchDrag::default();
        down(&mut t, Pos2::ZERO);
        still(&mut t, Pos2::new(0.0, -20.0), FRAME as f64);
        assert!(t.panning());
        assert_eq!(t.step(2.0 * f64::from(FRAME), true, true, Some(Pos2::new(0.0, -30.0)), Some(0.0), FRAME, LONG, true), Vec2::ZERO);
        assert!(t.pinching() && !t.panning() && !t.down());
        // One finger left after the pinch must not start panning again.
        assert_eq!(still(&mut t, Pos2::new(0.0, -60.0), 3.0 * f64::from(FRAME)), Vec2::ZERO);
        assert!(!t.panning());
        assert_eq!(t.step(4.0 * f64::from(FRAME), false, false, None, None, FRAME, LONG, true), Vec2::ZERO);
        assert!(!t.pinching());
    }

    #[test]
    fn a_web_press_is_adopted_by_the_touch_start_a_frame_later() {
        let mut t = TouchDrag::default();
        t.arm(Pos2::new(100.0, 100.0), 0.0, selection());
        assert!(!t.down());
        assert!(t.adopt(Pos2::new(102.0, 101.0), 0.5 * f64::from(FRAME)));
        assert!(t.down());
        // The press adopted is the finger's, so moving pans.
        let d = still(&mut t, Pos2::new(100.0, 60.0), 2.0 * f64::from(FRAME));
        assert!(t.panning() && d.y < 0.0);

        // A Touch Start elsewhere (another finger, another tap) is not this press.
        let mut t = TouchDrag::default();
        t.arm(Pos2::new(100.0, 100.0), 0.0, selection());
        assert!(!t.adopt(Pos2::new(300.0, 300.0), 0.5 * f64::from(FRAME)));
        assert!(!t.down());
    }

    #[test]
    fn hostile_numbers_never_panic_or_scroll_nonsense() {
        let mut t = TouchDrag::default();
        t.press(Pos2::new(f32::NAN, f32::INFINITY), selection());
        for (now, touches, multi, pointer, start, dt, long) in [
            (f64::NAN, true, false, Some(Pos2::new(f32::NAN, 0.0)), Some(f64::NAN), f32::NAN, f64::NAN),
            (0.0, true, false, Some(Pos2::new(f32::INFINITY, f32::NEG_INFINITY)), Some(0.0), f32::INFINITY, LONG),
            (1.0, true, false, None, Some(0.0), -1.0, LONG),
            (2.0, false, false, None, None, f32::NAN, LONG),
        ] {
            let d = t.step(now, touches, multi, pointer, start, dt, long, true);
            assert!(d.is_finite(), "{d:?}");
        }
        t.cancel();
        assert!(!t.down() && !t.panning() && !t.selecting() && !t.is_animating());
    }
}

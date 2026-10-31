//! Where the input method is told the caret is (its spot), on X11 not moved by typing.
//!
//! Over XIM, moving the spot is a request the app waits on (`XSetICValues`). A key the input method
//! forwards back to the app during that wait (a Vietnamese engine fixing a word sends BackSpaces,
//! then the new letters) is taken for a new key press and sent to the input method again: the app
//! gets it late, after the letters, and the engine sees a BackSpace nobody typed. The words come
//! out garbled (#2932).
//!
//! winit 0.30 makes the request when it handles the next X event, and the app learns of a key only
//! once the input method has handled it, so no moment chosen while typing is sure to be clear of a
//! key in flight (moving it once typing paused still garbled words typed about 350 ms apart). So
//! on X11 the spot stays where it is from the first key on and catches up with the caret at the
//! next click, tap or window focus; the request is then made at the next X event, a pointer event
//! or the next key press, before the input method has that key. Elsewhere the spot follows the
//! caret every frame.

use egui::Rect;

#[derive(Debug, Default)]
pub(crate) struct ImeSpot {
    /// The spot last given to the input method.
    shown: Option<Rect>,
    /// Keys came in since the last click, tap or window focus.
    typing: bool,
    /// Placements left that follow the caret whatever came in: a click's frame places the caret
    /// before the click moves it, so the next frame follows it too, keys or not.
    follow: u8,
    /// The last noted frame had a click and hasn't placed the caret yet.
    click_frame: bool,
}

impl ImeSpot {
    /// Note a frame's input, every frame (also one that doesn't place the caret): `keys` when it
    /// has key, text or input-method events, `pointer` when it has a click, a tap or the window's
    /// focus. Keys win in a frame with both.
    pub(crate) fn note(&mut self, keys: bool, pointer: bool) {
        // A click's frame that placed nothing (the canvas wasn't focused yet) leaves one follow:
        // the next placement already has the caret where the click put it, and a second one
        // would move the spot while the first keys come in.
        if std::mem::take(&mut self.click_frame) {
            self.follow = self.follow.min(1);
        }
        if pointer {
            self.follow = 2;
            self.click_frame = true;
        }
        if keys {
            self.typing = true;
        } else if pointer {
            self.typing = false;
        }
    }

    /// The spot for the caret at `caret`: the caret, except over XIM (`xim`) once keys have come
    /// in since the last click ([`ImeSpot::note`]), when it stays where it was. A click's frame and
    /// the one after it follow the caret either way.
    pub(crate) fn place(&mut self, caret: Rect, xim: bool) -> Rect {
        self.click_frame = false;
        let follow = self.follow > 0;
        self.follow = self.follow.saturating_sub(1);
        match self.shown {
            Some(shown) if xim && self.typing && !follow => shown,
            _ => {
                self.shown = Some(caret);
                caret
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn at(x: f32) -> Rect {
        Rect::from_min_size(pos2(x, 10.0), vec2(1.0, 16.0))
    }

    /// One frame with the caret at `x`: keys, a click, or neither.
    fn frame(s: &mut ImeSpot, x: f32, keys: bool, pointer: bool, xim: bool) -> Rect {
        s.note(keys, pointer);
        s.place(at(x), xim)
    }

    #[test]
    fn the_spot_follows_the_caret_off_x11() {
        let mut s = ImeSpot::default();
        for x in [10.0, 18.0, 26.0] {
            assert_eq!(frame(&mut s, x, true, false, false), at(x));
        }
    }

    /// #2932: over XIM typing doesn't move the spot, however long the pauses (a forwarded
    /// BackSpace counts as typing, also in a frame that doesn't place the caret); a click, tap or
    /// window focus does, until the next key.
    #[test]
    fn over_xim_typing_leaves_the_spot_until_a_click() {
        let mut s = ImeSpot::default();
        assert_eq!(frame(&mut s, 10.0, true, false, true), at(10.0), "the first spot");
        assert_eq!(frame(&mut s, 18.0, true, false, true), at(10.0), "typing");
        assert_eq!(frame(&mut s, 26.0, false, false, true), at(10.0), "a pause");
        s.note(true, false);
        assert_eq!(frame(&mut s, 34.0, false, false, true), at(10.0), "keys in a frame that didn't place the caret");
        assert_eq!(frame(&mut s, 90.0, false, true, true), at(90.0), "a click");
        assert_eq!(frame(&mut s, 120.0, false, false, true), at(120.0), "the caret settling after it");
        assert_eq!(frame(&mut s, 128.0, true, false, true), at(120.0), "typing again");
    }

    /// The frame of a click places the caret before the click moves it: the next frame follows
    /// the caret too, also when the first key after the click is already in it.
    #[test]
    fn the_frame_after_a_click_follows_the_caret_even_with_keys() {
        let mut s = ImeSpot::default();
        assert_eq!(frame(&mut s, 10.0, true, false, true), at(10.0));
        assert_eq!(frame(&mut s, 18.0, true, false, true), at(10.0), "typing");
        assert_eq!(frame(&mut s, 18.0, false, true, true), at(18.0), "the click's frame, caret not moved yet");
        assert_eq!(frame(&mut s, 90.0, true, false, true), at(90.0), "the next frame, with a key");
        assert_eq!(frame(&mut s, 98.0, true, false, true), at(90.0), "typing holds it again");
        assert_eq!(frame(&mut s, 98.0, true, true, true), at(98.0), "a frame with a click and keys");
    }

    /// A click on a canvas that isn't focused places nothing in its frame: the first placement
    /// after it follows the caret, and the first keys after that leave the spot (#2932 review).
    #[test]
    fn a_click_that_places_nothing_leaves_one_follow() {
        let mut s = ImeSpot::default();
        assert_eq!(frame(&mut s, 10.0, true, false, true), at(10.0));
        assert_eq!(frame(&mut s, 18.0, true, false, true), at(10.0), "typing");
        s.note(false, true);
        s.note(false, false);
        assert_eq!(frame(&mut s, 90.0, false, false, true), at(90.0), "the first placement, now focused");
        assert_eq!(frame(&mut s, 98.0, true, false, true), at(90.0), "the first key leaves it");
    }
}

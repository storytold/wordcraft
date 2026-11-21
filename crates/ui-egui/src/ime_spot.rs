//! Where the input method is told the caret is (its spot), on X11 held while typing.
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
//! on X11 the spot stays where it is while typing text, and catches up with the caret on a click,
//! tap, window or canvas focus, keyboard caret move (`Enter`, `Tab`, arrows, `Home`/`End`,
//! `PageUp`/`PageDown`), or when an input method starts or commits a pre-edit composition (which CJK engines
//! use and BackSpace-forwarding Vietnamese engines do not). Elsewhere the spot follows the caret
//! every frame.

use egui::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpotInput {
    /// Text, editing key, or input-method input that should hold the spot once placed.
    pub(crate) keys: bool,
    /// Pointer press, touch, or window focus: the caret moves after this frame's layout, so this
    /// placement and the next one follow it.
    pub(crate) pointer: bool,
    /// Keyboard caret move (`Enter`, `Tab`, arrows, `Home`/`End`, `PageUp`/`PageDown`) or
    /// composition start/commit: this frame's placement already has the new caret and catches up.
    pub(crate) move_caret: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ImeSpot {
    /// The spot last given to the input method.
    shown: Option<Rect>,
    /// Typing keys came in since the last click, focus, or caret-moving key.
    typing: bool,
    /// Currently inside a non-empty IME pre-edit composition.
    in_preedit: bool,
    /// Pre-edit was just cleared; the matching `Commit` may arrive in the same or next frame.
    pending_commit: bool,
    /// Placements left that follow the caret whatever came in: a click's frame places the caret
    /// before the click moves it, so the next frame follows it too, keys or not.
    follow: u8,
    /// The last noted frame had a click and hasn't placed the caret yet.
    click_frame: bool,
    /// Frame number last passed to `note_frame` / `place_frame`, so discarded layout passes in the
    /// same frame do not consume `follow` or re-note identical events.
    noted_frame: Option<u64>,
    placed_frame: Option<u64>,
}

fn is_caret_move_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::Enter
            | egui::Key::Tab
            | egui::Key::ArrowLeft
            | egui::Key::ArrowRight
            | egui::Key::ArrowUp
            | egui::Key::ArrowDown
            | egui::Key::Home
            | egui::Key::End
            | egui::Key::PageUp
            | egui::Key::PageDown
    )
}

impl ImeSpot {
    /// Classify a frame's egui events and note them once per `frame_nr`.
    pub(crate) fn note_frame(&mut self, frame_nr: u64, events: &[egui::Event]) {
        if self.noted_frame == Some(frame_nr) {
            return;
        }
        self.noted_frame = Some(frame_nr);
        self.note_events(events);
    }

    /// Note editor inputs already taken from egui events by the framework (`Preedit`, `ImeCommit`,
    /// `DeleteSurrounding`, caret moves, and text) before `page_input` runs. Leaves `click_frame`
    /// alone because `note_frame` already ran once for this frame.
    pub(crate) fn note_editor_inputs(&mut self, inputs: &[craft::behavior::editor::EditorInput]) {
        use craft::behavior::editor::EditorInput;
        let mut input = SpotInput::default();
        for e in inputs {
            match e {
                EditorInput::Preedit { text, .. } => {
                    input.keys = true;
                    if text.is_empty() {
                        self.in_preedit = false;
                        self.pending_commit = true;
                        input.move_caret = true;
                    } else if !self.in_preedit {
                        self.in_preedit = true;
                        self.pending_commit = false;
                        input.move_caret = true;
                    }
                }
                EditorInput::ImeCommit => {
                    input.keys = true;
                    if std::mem::take(&mut self.in_preedit) || std::mem::take(&mut self.pending_commit) {
                        input.move_caret = true;
                    }
                }
                EditorInput::Text(_) | EditorInput::Delete { .. } | EditorInput::DeleteSurrounding { .. } => {
                    input.keys = true;
                    self.pending_commit = false;
                }
                EditorInput::Move(..) | EditorInput::Newline | EditorInput::Tab => {
                    input.move_caret = true;
                }
                _ => {}
            }
        }
        if input != SpotInput::default() {
            if input.move_caret {
                self.follow = self.follow.max(1);
            }
            if input.keys {
                self.typing = true;
            } else if input.move_caret {
                self.typing = false;
            }
        }
    }

    /// Classify a frame's egui events and note them for the spot.
    pub(crate) fn note_events(&mut self, events: &[egui::Event]) {
        let mut input = SpotInput::default();
        for e in events {
            match e {
                egui::Event::PointerButton { pressed: true, .. } | egui::Event::Touch { .. } | egui::Event::WindowFocused(true) => {
                    input.pointer = true;
                }
                egui::Event::Key { key, pressed, .. } if is_caret_move_key(*key) => {
                    if *pressed {
                        input.move_caret = true;
                    }
                }
                egui::Event::Key { .. } | egui::Event::Text(_) => {
                    input.keys = true;
                    self.pending_commit = false;
                }
                egui::Event::Ime(ime) => match ime {
                    egui::ImeEvent::Preedit { text, .. } => {
                        input.keys = true;
                        if text.is_empty() {
                            self.in_preedit = false;
                            self.pending_commit = true;
                            input.move_caret = true;
                        } else if !self.in_preedit {
                            self.in_preedit = true;
                            self.pending_commit = false;
                            input.move_caret = true;
                        }
                    }
                    egui::ImeEvent::Commit(_) => {
                        input.keys = true;
                        if std::mem::take(&mut self.in_preedit) || std::mem::take(&mut self.pending_commit) {
                            input.move_caret = true;
                        }
                    }
                    egui::ImeEvent::DeleteSurrounding { .. } | egui::ImeEvent::DeleteSurroundingBytes { .. } => {
                        input.keys = true;
                    }
                    _ => {
                        self.in_preedit = false;
                        self.pending_commit = false;
                    }
                },
                _ => {}
            }
        }
        self.note(input);
    }

    /// Canvas regained keyboard focus (e.g. closing Find): let the next placement catch up.
    pub(crate) fn on_focus(&mut self) {
        self.typing = false;
        self.in_preedit = false;
        self.pending_commit = false;
    }

    /// Note a frame's input, every frame (also one that doesn't place the caret).
    pub(crate) fn note(&mut self, input: SpotInput) {
        // A click's frame that placed nothing (the canvas wasn't focused yet) leaves one follow:
        // the next placement already has the caret where the click put it, and a second one
        // would move the spot while the first keys come in.
        if std::mem::take(&mut self.click_frame) {
            self.follow = self.follow.min(1);
        }
        if input.pointer {
            self.follow = 2;
            self.click_frame = true;
        } else if input.move_caret {
            self.follow = self.follow.max(1);
        }
        if input.keys {
            self.typing = true;
        } else if input.pointer || input.move_caret {
            self.typing = false;
        }
    }

    /// Place the spot at most once per `frame_nr`, returning the same spot on discarded-layout
    /// reruns in the same frame so `follow` is not consumed twice.
    pub(crate) fn place_frame(&mut self, frame_nr: u64, caret: Rect, xim: bool) -> Rect {
        if self.placed_frame == Some(frame_nr)
            && let Some(shown) = self.shown
        {
            return shown;
        }
        self.placed_frame = Some(frame_nr);
        self.place(caret, xim)
    }

    /// The spot for the caret at `caret`: the caret, except over XIM (`xim`) while typing
    /// ([`ImeSpot::note`]), when it stays where it was. A click's frame and the one after it,
    /// keyboard caret moves, and composition starts follow the caret either way.
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
    use egui::{Event, ImeEvent, Key, Modifiers, pos2, vec2};

    fn at(x: f32) -> Rect {
        Rect::from_min_size(pos2(x, 10.0), vec2(1.0, 16.0))
    }

    /// One frame with the caret at `x`: keys, a click, or neither.
    fn frame(s: &mut ImeSpot, x: f32, keys: bool, pointer: bool, xim: bool) -> Rect {
        s.note(SpotInput { keys, pointer, move_caret: false });
        s.place(at(x), xim)
    }

    fn event_frame(s: &mut ImeSpot, x: f32, events: &[Event], xim: bool) -> Rect {
        s.note_events(events);
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
        s.note(SpotInput { keys: true, pointer: false, move_caret: false });
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
        s.note(SpotInput { keys: false, pointer: true, move_caret: false });
        s.note(SpotInput::default());
        assert_eq!(frame(&mut s, 90.0, false, false, true), at(90.0), "the first placement, now focused");
        assert_eq!(frame(&mut s, 98.0, true, false, true), at(90.0), "the first key leaves it");
    }

    /// #2932 review: Enter and caret-moving keys (arrows, Home/End, PageUp/PageDown) catch up the
    /// spot so CJK composition after Enter starts on the new line, while BackSpace still holds it.
    #[test]
    fn over_xim_enter_and_nav_keys_catch_up_while_backspace_holds() {
        let mut s = ImeSpot::default();
        let key = |k, pressed| Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        assert_eq!(event_frame(&mut s, 10.0, &[Event::Text("a".into())], true), at(10.0));
        assert_eq!(event_frame(&mut s, 18.0, &[Event::Text("b".into())], true), at(10.0), "plain typing holds spot");
        assert_eq!(event_frame(&mut s, 14.0, &[key(Key::Backspace, true)], true), at(10.0), "Backspace holds spot");
        assert_eq!(event_frame(&mut s, 50.0, &[key(Key::Enter, true)], true), at(50.0), "Enter updates spot");
        assert_eq!(event_frame(&mut s, 50.0, &[key(Key::Enter, false)], true), at(50.0), "Enter release stays ready");
        assert_eq!(
            event_frame(&mut s, 56.0, &[Event::Ime(ImeEvent::Preedit { text: "n".into(), active_range_chars: None })], true),
            at(56.0),
            "composing after Enter places candidate window on the new line"
        );
        assert_eq!(
            event_frame(&mut s, 62.0, &[Event::Ime(ImeEvent::Preedit { text: "ni".into(), active_range_chars: None })], true),
            at(56.0),
            "subsequent pre-edit keys keep spot steady"
        );
        assert_eq!(event_frame(&mut s, 70.0, &[Event::Ime(ImeEvent::Commit("你".into()))], true), at(70.0), "commit after pre-edit advances spot");
        assert_eq!(event_frame(&mut s, 40.0, &[key(Key::ArrowLeft, true)], true), at(40.0), "arrow key catches up");
        assert_eq!(event_frame(&mut s, 120.0, &[key(Key::Tab, true)], true), at(120.0), "Tab key catches up");
    }

    /// #2932 review: starting a pre-edit composition after typing plain words catches up the spot
    /// once, and committing that pre-edit advances the spot for the next composition while
    /// no-preedit commits (Vietnamese XIM) still hold it.
    #[test]
    fn over_xim_starting_and_committing_preedit_advances_spot() {
        let mut s = ImeSpot::default();
        assert_eq!(event_frame(&mut s, 10.0, &[Event::Text("a".into())], true), at(10.0));
        assert_eq!(event_frame(&mut s, 18.0, &[Event::Ime(ImeEvent::Commit("á".into()))], true), at(10.0), "no-preedit commit holds spot");
        assert_eq!(event_frame(&mut s, 26.0, &[Event::Text("b ".into())], true), at(10.0), "typing holds spot");
        assert_eq!(
            event_frame(&mut s, 90.0, &[Event::Ime(ImeEvent::Preedit { text: "n".into(), active_range_chars: None })], true),
            at(90.0),
            "entering pre-edit catches up"
        );
        assert_eq!(
            event_frame(&mut s, 98.0, &[Event::Ime(ImeEvent::Preedit { text: "ni".into(), active_range_chars: None })], true),
            at(90.0),
            "continuing pre-edit keeps spot steady"
        );
        assert_eq!(
            event_frame(&mut s, 106.0, &[Event::Ime(ImeEvent::Preedit { text: String::new(), active_range_chars: None })], true),
            at(106.0),
            "clearing pre-edit before commit catches up"
        );
        assert_eq!(
            event_frame(&mut s, 112.0, &[Event::Ime(ImeEvent::Commit("你".into()))], true),
            at(112.0),
            "commit right after cleared pre-edit advances spot for the next word"
        );
        assert_eq!(
            event_frame(&mut s, 118.0, &[Event::Ime(ImeEvent::Preedit { text: "h".into(), active_range_chars: None })], true),
            at(118.0),
            "next composition opens at the updated spot"
        );
    }

    /// Over XIM with fcitx5 candidate-popup engines, `winit` emits `Preedit("")` + `Commit`
    /// without any non-empty `Preedit`; that commit still advances the spot for the next word.
    #[test]
    fn over_xim_empty_preedit_commit_advances_spot() {
        let mut s = ImeSpot::default();
        assert_eq!(event_frame(&mut s, 10.0, &[Event::Text("a".into())], true), at(10.0));
        assert_eq!(
            event_frame(
                &mut s,
                42.0,
                &[Event::Ime(ImeEvent::Preedit { text: String::new(), active_range_chars: None }), Event::Ime(ImeEvent::Commit("你好".into())),],
                true,
            ),
            at(42.0),
            "XIM pre-edit-end commit advances spot"
        );
        assert_eq!(
            event_frame(&mut s, 74.0, &[Event::Ime(ImeEvent::Commit("世界".into()))], true),
            at(42.0),
            "a later no-preedit commit still holds spot"
        );
    }
}

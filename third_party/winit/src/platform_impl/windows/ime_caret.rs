//! When a window keeps a hidden system caret at the IME cursor area.
//!
//! Some IMEs place their composition and candidate windows at the system caret
//! (`GetCaretPos`, `GetGUIThreadInfo`) and ignore `ImmSetCandidateWindow`: Sogou, and some
//! Japanese IMEs. A window that has the keyboard focus and allows IME keeps a hidden caret over
//! the last IME cursor area. Windows has one caret per thread, and a window must destroy it before
//! it loses the focus.
//!
//! This is plain Rust so that its tests run on every platform (`tests/ime_patches.rs`).

/// An IME cursor area in physical client coordinates. Both sides are at least 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CaretArea {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl CaretArea {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        CaretArea { x, y, width: width.max(1), height: height.max(1) }
    }
}

/// What to do with the thread's system caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaretChange {
    /// `CreateCaret` at this size, then `SetCaretPos`. It replaces any caret the thread has.
    Create(CaretArea),
    /// `SetCaretPos`.
    Move(CaretArea),
    /// `DestroyCaret`.
    Destroy,
}

#[derive(Debug, Default)]
pub(crate) struct SystemCaret {
    focused: bool,
    ime_allowed: bool,
    area: Option<CaretArea>,
    /// The caret this window has created, if any.
    current: Option<CaretArea>,
}

impl SystemCaret {
    pub fn set_focused(&mut self, focused: bool) -> Option<CaretChange> {
        self.focused = focused;
        self.update()
    }

    pub fn set_ime_allowed(&mut self, allowed: bool) -> Option<CaretChange> {
        self.ime_allowed = allowed;
        self.update()
    }

    pub fn set_area(&mut self, area: CaretArea) -> Option<CaretChange> {
        self.area = Some(area);
        self.update()
    }

    fn update(&mut self) -> Option<CaretChange> {
        let wanted = self.area.filter(|_| self.focused && self.ime_allowed);
        match (std::mem::replace(&mut self.current, wanted), wanted) {
            (None, None) => None,
            (Some(_), None) => Some(CaretChange::Destroy),
            (None, Some(new)) => Some(CaretChange::Create(new)),
            (Some(old), Some(new)) if old == new => None,
            (Some(old), Some(new)) if (old.width, old.height) == (new.width, new.height) => {
                Some(CaretChange::Move(new))
            },
            (Some(_), Some(new)) => Some(CaretChange::Create(new)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_exists_only_while_focused_with_ime_allowed() {
        let area = CaretArea::new(10, 20, 2, 16);
        let mut caret = SystemCaret::default();
        assert_eq!(caret.set_area(area), None);
        assert_eq!(caret.set_focused(true), None);
        assert_eq!(caret.set_ime_allowed(true), Some(CaretChange::Create(area)));
        assert_eq!(caret.set_focused(false), Some(CaretChange::Destroy));
        assert_eq!(caret.set_focused(true), Some(CaretChange::Create(area)));
        assert_eq!(caret.set_ime_allowed(false), Some(CaretChange::Destroy));
        assert_eq!(caret.set_ime_allowed(false), None);
    }

    #[test]
    fn caret_waits_for_a_cursor_area() {
        let mut caret = SystemCaret::default();
        assert_eq!(caret.set_focused(true), None);
        assert_eq!(caret.set_ime_allowed(true), None);
        let area = CaretArea::new(0, 0, 1, 18);
        assert_eq!(caret.set_area(area), Some(CaretChange::Create(area)));
    }

    #[test]
    fn caret_follows_the_cursor_area() {
        let mut caret = SystemCaret::default();
        caret.set_focused(true);
        caret.set_ime_allowed(true);
        let first = CaretArea::new(10, 20, 1, 16);
        assert_eq!(caret.set_area(first), Some(CaretChange::Create(first)));
        assert_eq!(caret.set_area(first), None);
        let moved = CaretArea::new(30, 20, 1, 16);
        assert_eq!(caret.set_area(moved), Some(CaretChange::Move(moved)));
        // A caret's size is fixed when it is created.
        let taller = CaretArea::new(30, 20, 1, 24);
        assert_eq!(caret.set_area(taller), Some(CaretChange::Create(taller)));
    }

    #[test]
    fn caret_area_is_never_empty() {
        assert_eq!(CaretArea::new(5, 6, 0, -3), CaretArea { x: 5, y: 6, width: 1, height: 1 });
    }
}

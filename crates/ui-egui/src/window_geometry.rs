//! The main window's size, position and maximized state, saved with the UI preferences so the
//! next launch opens the window where the user left it (#23).
//!
//! Sizes and positions are in egui points: what [`egui::ViewportInfo`] reports and what
//! [`egui::ViewportBuilder`] takes.

use serde::{Deserialize, Serialize};

/// The host's window state for one raw frame.
///
/// [`egui::InputState::viewport`] still describes the last painted frame on a hidden logic tick,
/// and its `viewport_rect()` is that frame's content size. The host's [`egui::RawInput`] is the
/// fresh answer for this frame, so the app copies the fields it needs here in `raw_input_hook`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawWindowState {
    /// The content rectangle, when the platform reports it (not Wayland).
    pub inner_rect: Option<egui::Rect>,
    /// The window rectangle including decorations, when the platform reports it.
    pub outer_rect: Option<egui::Rect>,
    /// This frame's `RawInput::screen_rect`: the content size even where `inner_rect` is unknown.
    pub screen_rect: Option<egui::Rect>,
    pub monitor_size: Option<egui::Vec2>,
    pub maximized: Option<bool>,
    pub minimized: Option<bool>,
    pub fullscreen: Option<bool>,
}

impl RawWindowState {
    /// Copy the window state out of this frame's raw input.
    pub fn from_raw(raw: &egui::RawInput) -> Self {
        Self::from_viewport(raw.viewports.get(&raw.viewport_id), raw.screen_rect)
    }

    /// The state of one viewport, from what egui knows about it and its content rectangle.
    pub fn from_viewport(viewport: Option<&egui::ViewportInfo>, screen_rect: Option<egui::Rect>) -> Self {
        Self {
            inner_rect: viewport.and_then(|v| v.inner_rect).filter(|r| r.is_finite()),
            outer_rect: viewport.and_then(|v| v.outer_rect).filter(|r| r.is_finite()),
            screen_rect: screen_rect.filter(|r| r.is_finite()),
            monitor_size: viewport.and_then(|v| v.monitor_size),
            maximized: viewport.and_then(|v| v.maximized),
            minimized: viewport.and_then(|v| v.minimized),
            fullscreen: viewport.and_then(|v| v.fullscreen),
        }
    }

    /// The content size reported by this frame, if any. Never the last painted size: a hidden
    /// Wayland tick with neither an `inner_rect` nor a screen rect has no usable geometry.
    pub fn content_size(self) -> Option<egui::Vec2> {
        [self.inner_rect, self.screen_rect]
            .into_iter()
            .flatten()
            .filter(|r| r.is_finite())
            .map(|r| r.size())
            .find(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)
    }
}

/// The smallest window restored: the desktop app's minimum inner size.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(760.0, 480.0);
/// Larger saved sizes are corrupt. eframe also shrinks the start size to the largest monitor.
const MAX_SIZE: egui::Vec2 = egui::vec2(16384.0, 16384.0);
/// Saved positions farther from the origin than this are corrupt.
const MAX_COORD: f32 = 32768.0;
/// Room a window leaves on each side of its monitor for the taskbar, the Dock or a desktop panel,
/// which the monitor size (all egui reports) includes.
const SYSTEM_BAR: egui::Vec2 = egui::vec2(48.0, 48.0);

/// The window as it was when the app last closed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowGeometry {
    /// Top-left corner: of the frame, or of the content on macOS (where winit places a new
    /// window by it). `None` where the platform doesn't report it (Wayland).
    pub position: Option<[f32; 2]>,
    /// Content size of the un-maximized window.
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
    /// Size of the monitor the window was on, to notice when that monitor is gone.
    pub monitor: Option<[f32; 2]>,
}

impl WindowGeometry {
    /// The geometry to save, given the last saved one and the window as it is now. While the
    /// window is minimized, full screen or maximized, the last un-maximized size and position are
    /// kept, so un-maximizing after the next launch returns to them.
    pub fn track(prev: Option<Self>, window: &RawWindowState) -> Option<Self> {
        if window.minimized == Some(true) || window.fullscreen == Some(true) {
            return prev;
        }
        // egui reads the maximized state on macOS only when the window opens (egui#3494), so a
        // window that opened zoomed would stay "maximized" for good. There the zoomed frame is
        // saved like any other size instead.
        let maximized = window.maximized == Some(true) && !cfg!(target_os = "macos");
        if maximized && let Some(prev) = prev {
            return Some(Self { maximized, ..prev });
        }
        let rect = if cfg!(target_os = "macos") { window.inner_rect } else { window.outer_rect };
        // No rect on this raw frame: keep the last saved geometry. Falling back to
        // `InputState::viewport_rect()` here would save the previous painted size, which is the
        // stale value a hidden Wayland tick must not write.
        let Some(size) = window.content_size() else {
            return prev;
        };
        let position = rect.map(|r| [r.min.x, r.min.y]);
        let monitor = window.monitor_size.map(|s| [s.x, s.y]);
        Self { position, width: size.x, height: size.y, maximized, monitor }.sanitized().or(prev)
    }

    /// `None` for a size that isn't positive and finite. Otherwise the size is clamped to
    /// [`MIN_SIZE`]..=16384, and a position or monitor size that can't be real is dropped.
    pub fn sanitized(self) -> Option<Self> {
        let size = egui::vec2(self.width, self.height);
        if !size.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
            return None;
        }
        let size = size.clamp(MIN_SIZE, MAX_SIZE);
        let position = self.position.filter(|p| p.iter().all(|c| c.is_finite() && c.abs() <= MAX_COORD));
        let monitor = self.monitor.filter(|m| m.iter().all(|c| c.is_finite() && *c > 0.0 && *c <= MAX_SIZE.x));
        Some(Self { position, width: size.x, height: size.y, maximized: self.maximized, monitor })
    }

    /// Whether the window is maximized after it has opened rather than as it opens. On Windows,
    /// winit shows a window the moment it creates it maximized, even one that is meant to stay
    /// hidden until the first frame (eframe's white-flash guard), so a maximized window flashed
    /// on and off about four times at start-up. There it opens at its normal size, hidden, and the
    /// first frame maximizes it ([`Self::maximize_after_open`]).
    pub fn maximizes_after_open(self) -> bool {
        self.maximized && cfg!(windows)
    }

    /// The command that maximizes the window after it opened, if [`Self::maximizes_after_open`].
    pub fn maximize_after_open(self) -> Option<egui::ViewportCommand> {
        self.maximizes_after_open().then_some(egui::ViewportCommand::Maximized(true))
    }

    /// Open the window at this size, position and maximized state.
    pub fn apply(self, builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
        let builder = builder.with_inner_size([self.width, self.height]).with_maximized(self.maximized && !self.maximizes_after_open());
        match self.position {
            Some(p) => builder.with_position(p),
            None => builder,
        }
    }

    /// After a restore: where to move the window when the monitor it now reports differs from the
    /// one it was saved on (an external monitor was unplugged, say), so a window restored to a
    /// position no monitor covers any more doesn't open off screen. Centred on the primary
    /// monitor, whose top-left corner is the origin. A maximized window fills whichever monitor
    /// the system picks, so it stays put.
    pub fn rescue_position(&self, window: &RawWindowState) -> Option<egui::Pos2> {
        if self.maximized || self.position.is_none() {
            return None;
        }
        let saved = egui::Vec2::from(self.monitor?);
        let now = window.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)?;
        if (now - saved).length() <= 1.0 {
            return None;
        }
        let free = now - egui::vec2(self.width, self.height);
        Some(egui::pos2((free.x / 2.0).max(0.0), (free.y / 2.0).max(0.0)))
    }

    /// Once the monitor size is known: a smaller size and a centred position for a window that
    /// doesn't fit on its monitor, frame included (#1215). eframe shrinks the start size to the
    /// monitor's, but not by the frame, the title bar or the taskbar, so on a laptop screen the
    /// default window reached past the screen's edge, and the end of the scroll bar with it: a
    /// drag of the scroll bar stopped where the pointer met the edge. A window the system placed
    /// (no position here) is on the primary monitor, whose top-left corner is the origin, so its
    /// position counts too; a restored one may be on another monitor, so only its size does. A
    /// maximized window fills its monitor already.
    pub fn fit_on_monitor(&self, window: &RawWindowState) -> Option<(egui::Pos2, egui::Vec2)> {
        if self.maximized || window.maximized == Some(true) || window.fullscreen == Some(true) || window.minimized == Some(true) {
            return None;
        }
        let monitor = window.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)?;
        let inner = window.inner_rect.filter(|r| r.is_finite())?;
        let outer = window.outer_rect.filter(|r| r.is_finite()).unwrap_or(inner);
        let frame = (outer.size() - inner.size()).max(egui::Vec2::ZERO);
        let room = monitor - 2.0 * SYSTEM_BAR;
        let too_big = outer.width() > room.x || outer.height() > room.y;
        let edge = monitor - SYSTEM_BAR;
        let past_edge = self.position.is_none() && (outer.max.x > edge.x || outer.max.y > edge.y);
        if !too_big && !past_edge {
            return None;
        }
        let size = (room - frame).min(inner.size()).max(MIN_SIZE);
        let free = monitor - (size + frame);
        Some((egui::pos2((free.x / 2.0).max(0.0), (free.y / 2.0).max(0.0)), size))
    }
}

/// Checks the restored geometry in `pending` once, and only once the monitor size is known: while
/// `window` doesn't report a usable one, `pending` is left in place for a later frame. Then it is taken
/// and [`WindowGeometry::rescue_position`] decides where to move the window, if anywhere.
pub fn take_rescue(pending: &mut Option<WindowGeometry>, window: &RawWindowState) -> Option<egui::Pos2> {
    window.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)?;
    pending.take()?.rescue_position(window)
}

/// The window size to ask for when a caller (the control channel's `ui.resize`) wants `size`:
/// `None` unless both sides are positive and finite. Otherwise at least [`MIN_SIZE`], the desktop
/// app's minimum, and at most `max_side` points (the largest texture the GPU draws, which the
/// window's surface must fit in) or 16384, whichever is smaller; the GPU's limit wins over the
/// minimum (#3734).
pub fn requested_size(size: egui::Vec2, max_side: f32) -> Option<egui::Vec2> {
    if !size.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let max = MAX_SIZE.min(egui::Vec2::splat(max_side));
    if !(max.x > 0.0 && max.y > 0.0) {
        return None;
    }
    Some(size.max(MIN_SIZE).min(max))
}

/// The monitors the host reported last, in physical pixels of the desktop's coordinate space
/// (the one winit places windows in). Kept in the context, so a dialog drawn in any window finds them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Monitors(Vec<egui::Rect>);

fn monitors_key() -> egui::Id {
    egui::Id::unique("window_geometry::monitors")
}

/// The host's monitors, in physical pixels, each frame: the desktop app reads them from winit. A
/// host that doesn't know them (the web app, tests) leaves them unset, and dialogs are centred on
/// the window as before.
pub fn set_monitors(ctx: &egui::Context, monitors: Vec<egui::Rect>) {
    let monitors = Monitors(monitors);
    if ctx.data(|d| d.get_temp::<Monitors>(monitors_key()).as_ref() != Some(&monitors)) {
        ctx.data_mut(|d| d.insert_temp(monitors_key(), monitors));
    }
}

/// The host's monitors in this viewport's points, in the coordinates of
/// [`egui::ViewportInfo::inner_rect`] (which egui-winit divides by the same pixels per point).
pub fn monitors_in_points(ctx: &egui::Context) -> Vec<egui::Rect> {
    let ppp = ctx.pixels_per_point();
    if !(ppp.is_finite() && ppp > 0.0) {
        return Vec::new();
    }
    ctx.data(|d| d.get_temp::<Monitors>(monitors_key())).map(|m| m.0.into_iter().map(|r| r / ppp).collect()).unwrap_or_default()
}

/// Where a dialog goes instead of its usual place, when that place is off screen (#4842).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogPlacement {
    /// How far to move the dialog from its usual place, in points.
    pub dialog_shift: egui::Vec2,
    /// How far to move the window so the dialog is on screen, in points: zero unless too little
    /// of the window is on its monitor to hold the dialog. A dialog is drawn inside its window, so
    /// a window almost or entirely off screen can only show it by coming back on screen.
    pub window_shift: egui::Vec2,
}

/// Placing a dialog that would be at `dialog` (in desktop points) in a window whose content is
/// at `content`, on `monitors` (the same coordinates; negative for monitors left of or above the
/// primary one). `None` (no change) when the dialog is all on one monitor already, as for every
/// window that is on screen, and when the geometry is unknown or not sane.
///
/// Otherwise, on the monitor that shows most of the window (the nearest when none does), the
/// dialog is centred on the part of the window that the monitor shows. Along an axis where that
/// part is too small for it, the dialog goes to the window's edge nearest the monitor and the
/// window moves just far enough to bring it on screen.
pub fn place_dialog_on_screen(content: egui::Rect, monitors: &[egui::Rect], dialog: egui::Rect) -> Option<DialogPlacement> {
    let sane =
        |r: &egui::Rect| r.is_finite() && r.min.x.abs() <= MAX_COORD * 8.0 && r.min.y.abs() <= MAX_COORD * 8.0 && r.width() > 0.0 && r.height() > 0.0;
    if !sane(&content) || !sane(&dialog) {
        return None;
    }
    let monitors: Vec<egui::Rect> = monitors.iter().copied().filter(sane).collect();
    // On screen: each corner on a monitor (a window across two side-by-side monitors included).
    let on_screen = |p: egui::Pos2| monitors.iter().any(|m| m.contains(p));
    if [dialog.left_top(), dialog.right_top(), dialog.left_bottom(), dialog.right_bottom()].into_iter().all(on_screen) {
        return None;
    }
    let overlap = |m: &egui::Rect| {
        let i = m.intersect(content);
        if i.is_positive() { i.area() } else { 0.0 }
    };
    let distance = |m: &egui::Rect| m.distance_sq_to_pos(content.center());
    let shown = monitors.iter().copied().filter(|m| overlap(m) > 0.0).max_by(|a, b| overlap(a).total_cmp(&overlap(b)));
    let monitor = shown.or_else(|| monitors.iter().copied().min_by(|a, b| distance(a).total_cmp(&distance(b))))?;
    let (x, wx) = place_span(content.x_range(), monitor.x_range(), dialog.x_range());
    let (y, wy) = place_span(content.y_range(), monitor.y_range(), dialog.y_range());
    let placement = DialogPlacement { dialog_shift: egui::vec2(x - dialog.min.x, y - dialog.min.y), window_shift: egui::vec2(wx, wy) };
    (placement.dialog_shift != egui::Vec2::ZERO || placement.window_shift != egui::Vec2::ZERO).then_some(placement)
}

/// One axis of [`place_dialog_on_screen`]: the dialog's new start, and how far the window moves.
fn place_span(content: egui::Rangef, monitor: egui::Rangef, dialog: egui::Rangef) -> (f32, f32) {
    let size = dialog.span();
    let (lo, hi) = (content.min.max(monitor.min), content.max.min(monitor.max));
    if dialog.min >= lo && dialog.max <= hi {
        return (dialog.min, 0.0);
    }
    if hi - lo >= size {
        return (lo + (hi - lo - size) / 2.0, 0.0);
    }
    // Too little of the window is on the monitor: the dialog takes the window's edge nearest the
    // monitor, and the window brings that much of itself on screen. A dialog larger than the
    // window or the monitor shows its start (the title bar, the left edge).
    let room = size.min(content.span()).min(monitor.span());
    let start = if content.max - room < monitor.min { content.max - room } else { content.min };
    let start = start.max(content.min);
    let shift = if start < monitor.min {
        monitor.min - start
    } else if start + room > monitor.max {
        monitor.max - (start + room)
    } else {
        0.0
    };
    (start, shift)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a raw window-state fixture whose inner, outer and screen rectangles match and whose
    /// monitor is `monitor`.
    fn raw(rect: egui::Rect, monitor: egui::Vec2) -> RawWindowState {
        RawWindowState { inner_rect: Some(rect), outer_rect: Some(rect), screen_rect: Some(rect), monitor_size: Some(monitor), ..Default::default() }
    }

    fn saved() -> WindowGeometry {
        WindowGeometry { position: Some([100.0, 50.0]), width: 1200.0, height: 800.0, maximized: false, monitor: Some([1920.0, 1080.0]) }
    }

    /// Tracking a normal rectangle saves its position, size and monitor, and applies them to a
    /// viewport builder.
    #[test]
    fn records_the_window_and_restores_it() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1200.0, 800.0));
        let g = WindowGeometry::track(None, &raw(rect, egui::vec2(1920.0, 1080.0)));
        assert_eq!(g, Some(saved()));
        let b = saved().apply(egui::ViewportBuilder::default());
        assert_eq!(b.inner_size, Some(egui::vec2(1200.0, 800.0)));
        assert_eq!(b.position, Some(egui::pos2(100.0, 50.0)));
        assert_eq!(b.maximized, Some(false));
    }

    #[test]
    fn a_maximized_window_opens_normal_on_windows_and_is_maximized_by_the_first_frame() {
        let g = WindowGeometry { maximized: true, ..saved() };
        let b = g.apply(egui::ViewportBuilder::default());
        // Normal size and position either way, so un-maximizing returns to them.
        assert_eq!(b.inner_size, Some(egui::vec2(1200.0, 800.0)));
        assert_eq!(b.position, Some(egui::pos2(100.0, 50.0)));
        if cfg!(windows) {
            assert_eq!(b.maximized, Some(false), "winit would show the hidden window while creating it maximized");
            assert_eq!(g.maximize_after_open(), Some(egui::ViewportCommand::Maximized(true)));
        } else {
            assert_eq!(b.maximized, Some(true));
            assert_eq!(g.maximize_after_open(), None);
        }
        assert_eq!(saved().maximize_after_open(), None, "a normal window needs no command");
    }

    /// On macOS a zoomed frame is saved as its zoomed frame rectangle without the maximized flag;
    /// elsewhere the previous geometry is kept with the maximized state. Minimized and full-screen
    /// frames change nothing.
    #[test]
    fn maximized_keeps_the_normal_size_and_position() {
        let full = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1040.0));
        let mut i = raw(full, egui::vec2(1920.0, 1080.0));
        i.maximized = Some(true);
        let g = WindowGeometry::track(Some(saved()), &i);
        if cfg!(target_os = "macos") {
            // The zoomed frame is saved as the size; the window never reopens "maximized".
            assert_eq!(g.map(|g| (g.position, g.width, g.height, g.maximized)), Some((Some([0.0, 0.0]), 1920.0, 1040.0, false)));
        } else {
            assert_eq!(g, Some(WindowGeometry { maximized: true, ..saved() }));
        }
        // Minimized or full screen: nothing changes.
        i.maximized = Some(false);
        i.minimized = Some(true);
        assert_eq!(WindowGeometry::track(Some(saved()), &i), Some(saved()));
        i.minimized = None;
        i.fullscreen = Some(true);
        assert_eq!(WindowGeometry::track(Some(saved()), &i), Some(saved()));
    }

    /// With no position reported, the raw screen rectangle still supplies the size, and the restored
    /// builder has no position.
    #[test]
    fn without_a_position_the_raw_screen_size_is_kept() {
        // Wayland reports no inner/outer rect, but RawInput still carries the content screen rect.
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
        let i = RawWindowState { screen_rect: Some(rect), ..Default::default() };
        let g = WindowGeometry::track(None, &i);
        assert_eq!(g, Some(WindowGeometry { position: None, width: 1000.0, height: 700.0, maximized: false, monitor: None }));
        assert_eq!(g.map(|g| g.apply(egui::ViewportBuilder::default()).position), Some(None));
    }

    /// A hidden Wayland logic tick has no fresh rect. It must keep the saved geometry rather than
    /// write the previous painted `InputState` size, and a fresh `screen_rect` on the same raw
    /// frame is used when the host reports one.
    #[test]
    fn a_hidden_raw_tick_never_reuses_the_last_painted_size() {
        assert_eq!(WindowGeometry::track(Some(saved()), &RawWindowState::default()), Some(saved()));

        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
        let fresh = RawWindowState {
            inner_rect: None,
            outer_rect: None,
            screen_rect: Some(rect),
            monitor_size: Some(egui::vec2(1920.0, 1080.0)),
            ..Default::default()
        };
        let g = WindowGeometry::track(Some(saved()), &fresh).expect("the fresh raw size");
        assert_eq!((g.position, g.width, g.height, g.monitor), (None, 1000.0, 700.0, Some([1920.0, 1080.0])));
    }

    /// Invalid inner dimensions use fresh screen geometry; valid inner dimensions take precedence.
    #[test]
    fn invalid_inner_size_falls_back_to_fresh_screen_geometry() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
        for size in [egui::vec2(0.0, 0.0), egui::vec2(-1.0, 10.0), egui::vec2(f32::NAN, 10.0)] {
            let raw = RawWindowState {
                inner_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                screen_rect: Some(screen),
                ..Default::default()
            };
            assert_eq!(raw.content_size(), Some(screen.size()));
        }
        let inner = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        assert_eq!(RawWindowState { inner_rect: Some(inner), screen_rect: Some(screen), ..Default::default() }.content_size(), Some(inner.size()));
        assert_eq!(
            RawWindowState { inner_rect: Some(egui::Rect::ZERO), screen_rect: Some(egui::Rect::ZERO), ..Default::default() }.content_size(),
            None
        );
    }

    /// Only the current frame's root viewport contributes inner, outer, content and monitor sizes; a
    /// missing viewport or non-finite data leaves defaults.
    #[test]
    fn raw_window_state_reads_only_this_frames_viewport() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0));
        let mut input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
        input.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                inner_rect: Some(rect),
                outer_rect: Some(rect.expand(10.0)),
                monitor_size: Some(egui::vec2(1920.0, 1080.0)),
                maximized: Some(false),
                minimized: Some(false),
                fullscreen: Some(false),
                ..Default::default()
            },
        );
        let state = RawWindowState::from_raw(&input);
        assert_eq!(state.inner_rect, Some(rect));
        assert_eq!(state.outer_rect, Some(rect.expand(10.0)));
        assert_eq!(state.content_size(), Some(egui::vec2(640.0, 480.0)));
        assert_eq!(state.monitor_size, Some(egui::vec2(1920.0, 1080.0)));

        // A frame without its viewport entry (or with non-finite rects) contributes nothing.
        let empty = RawWindowState::from_raw(&egui::RawInput::default());
        assert_eq!(empty, RawWindowState::default());
        assert_eq!(empty.content_size(), None);
    }

    #[test]
    fn insane_values_are_clamped_or_dropped() {
        let zero = WindowGeometry { width: 0.0, height: 800.0, ..saved() };
        assert_eq!(zero.sanitized(), None);
        assert_eq!(WindowGeometry::default().sanitized(), None);
        let nan = WindowGeometry { width: f32::NAN, ..saved() };
        assert_eq!(nan.sanitized(), None);
        let tiny = WindowGeometry { width: 10.0, height: 1.0e9, ..saved() }.sanitized();
        assert_eq!(tiny.map(|g| (g.width, g.height)), Some((MIN_SIZE.x, 16384.0)));
        let far = WindowGeometry { position: Some([1.0e7, 0.0]), monitor: Some([f32::INFINITY, 1080.0]), ..saved() }.sanitized();
        assert_eq!(far.map(|g| (g.position, g.monitor)), Some((None, None)));
        assert_eq!(saved().sanitized(), Some(saved()));
    }

    /// A saved window on a different monitor is centred on it, or clamped to its origin when too
    /// large; the same monitor, missing data, no saved position or a maximized window does nothing.
    #[test]
    fn a_window_whose_monitor_is_gone_is_moved_on_screen() {
        let on = egui::Rect::from_min_size(egui::pos2(2000.0, 50.0), egui::vec2(1200.0, 800.0));
        // Same monitor: stays where it was.
        assert_eq!(saved().rescue_position(&raw(on, egui::vec2(1920.0, 1080.0))), None);
        // A different (smaller) monitor now: centred on it.
        assert_eq!(saved().rescue_position(&raw(on, egui::vec2(1600.0, 900.0))), Some(egui::pos2(200.0, 50.0)));
        // Larger than that monitor: its top-left corner.
        assert_eq!(saved().rescue_position(&raw(on, egui::vec2(1024.0, 768.0))), Some(egui::Pos2::ZERO));
        // Nothing to compare, or no saved position: no move.
        assert_eq!(WindowGeometry { monitor: None, ..saved() }.rescue_position(&raw(on, egui::vec2(1600.0, 900.0))), None);
        assert_eq!(WindowGeometry { position: None, ..saved() }.rescue_position(&raw(on, egui::vec2(1600.0, 900.0))), None);
        assert_eq!(WindowGeometry { maximized: true, ..saved() }.rescue_position(&raw(on, egui::vec2(1600.0, 900.0))), None);
        assert_eq!(saved().rescue_position(&RawWindowState::default()), None);
    }

    /// The rescue waits for a known monitor size, then runs once (moving when needed) and clears the
    /// pending geometry.
    #[test]
    fn the_rescue_waits_for_the_monitor_size() {
        let on = egui::Rect::from_min_size(egui::pos2(2000.0, 50.0), egui::vec2(1200.0, 800.0));
        // The first frame doesn't know the monitor yet: nothing happens and the geometry is kept.
        let mut pending = Some(saved());
        assert_eq!(take_rescue(&mut pending, &RawWindowState::default()), None);
        assert_eq!(pending, Some(saved()));
        // A later frame reports a different monitor: centred on it, and the check is done.
        assert_eq!(take_rescue(&mut pending, &raw(on, egui::vec2(1600.0, 900.0))), Some(egui::pos2(200.0, 50.0)));
        assert_eq!(pending, None);
        assert_eq!(take_rescue(&mut pending, &raw(on, egui::vec2(1024.0, 768.0))), None);
        // The same monitor: no move, and the check is done as well.
        let mut pending = Some(saved());
        assert_eq!(take_rescue(&mut pending, &raw(on, egui::vec2(1920.0, 1080.0))), None);
        assert_eq!(pending, None);
    }

    #[test]
    fn survives_the_prefs_round_trip() {
        let ui = crate::UiState { window: Some(saved()), ..Default::default() };
        let json = serde_json::to_string(&ui).unwrap();
        let back: crate::UiState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.window, Some(saved()));
        // Prefs written before #23 have no window.
        let old: crate::UiState = serde_json::from_str(r#"{"tab":"Insert"}"#).unwrap();
        assert_eq!(old.window, None);
        assert_eq!(old.tab, "Insert");
    }

    /// A window opened with its frame's top-left corner at `at`: 8 pt borders, a 31 pt title bar.
    fn opened(at: egui::Pos2, content: egui::Vec2, monitor: egui::Vec2) -> RawWindowState {
        RawWindowState {
            inner_rect: Some(egui::Rect::from_min_size(at + egui::vec2(8.0, 31.0), content)),
            outer_rect: Some(egui::Rect::from_min_size(at, content + egui::vec2(16.0, 39.0))),
            monitor_size: Some(monitor),
            ..Default::default()
        }
    }

    #[test]
    fn requested_sizes_are_positive_and_fit_the_gpu() {
        let bad = [
            egui::vec2(-5.0, 600.0),
            egui::vec2(0.0, 0.0),
            egui::vec2(1200.0, f32::NAN),
            egui::vec2(f32::NEG_INFINITY, 600.0),
            egui::vec2(f32::INFINITY, 600.0),
        ];
        for bad in bad {
            assert_eq!(requested_size(bad, 8192.0), None, "{bad:?}");
        }
        assert_eq!(requested_size(egui::vec2(1200.0, 800.0), 8192.0), Some(egui::vec2(1200.0, 800.0)));
        assert_eq!(requested_size(egui::vec2(1e9, 1.0), 8192.0), Some(egui::vec2(8192.0, MIN_SIZE.y)));
        assert_eq!(requested_size(egui::vec2(1e9, 1e9), f32::INFINITY), Some(MAX_SIZE));
        assert_eq!(requested_size(egui::vec2(1e9, 1e9), f32::NAN), Some(MAX_SIZE));
        // A GPU that can't hold the minimum: its limit wins, and no limit at all refuses.
        assert_eq!(requested_size(egui::vec2(1200.0, 300.0), 600.0), Some(egui::vec2(600.0, 480.0)));
        assert_eq!(requested_size(egui::vec2(1200.0, 300.0), 100.0), Some(egui::vec2(100.0, 100.0)));
        assert_eq!(requested_size(egui::vec2(1200.0, 800.0), 0.0), None);
    }

    /// An oversized default window shrinks and is centred to fit, while a restored window keeps its
    /// place; maximized, full-screen, minimized and unknown-monitor frames, and frames without
    /// rectangles, are left alone.
    #[test]
    fn a_window_too_big_for_its_screen_shrinks_to_fit() {
        let system = WindowGeometry::default();
        let at = egui::pos2(78.0, 78.0);
        // A 1920 × 1080 laptop screen at 125 %: eframe gave the default window the screen's height,
        // so the frame and the title bar push its bottom, and the scroll bar's end, off screen.
        let laptop = egui::vec2(1536.0, 864.0);
        let fitted = Some((egui::pos2(48.0, 48.0), egui::vec2(1424.0, 729.0)));
        assert_eq!(system.fit_on_monitor(&opened(at, egui::vec2(1440.0, 864.0), laptop)), fitted);
        // The same size restored from an earlier launch shrinks too.
        assert_eq!(saved().fit_on_monitor(&opened(at, egui::vec2(1440.0, 864.0), laptop)), fitted);
        // Full HD: the default size fits, but where the system put it the bottom reaches into the
        // taskbar, so it is only centred.
        let full_hd = egui::vec2(1920.0, 1080.0);
        let start = egui::vec2(1440.0, 920.0);
        assert_eq!(system.fit_on_monitor(&opened(at, start, full_hd)), Some((egui::pos2(232.0, 60.5), start)));
        // A restored window keeps its place: it may be on another monitor (its origin isn't known).
        assert_eq!(saved().fit_on_monitor(&opened(at, start, full_hd)), None);
        assert_eq!(saved().fit_on_monitor(&opened(egui::pos2(2000.0, 78.0), start, full_hd)), None);
        // Room to spare: nothing moves.
        assert_eq!(system.fit_on_monitor(&opened(at, start, egui::vec2(2560.0, 1440.0))), None);
        // Once fitted, the window saved and restored on the next launch stays as it is.
        let again = WindowGeometry { position: Some([48.0, 48.0]), width: 1424.0, height: 729.0, ..saved() };
        assert_eq!(again.fit_on_monitor(&opened(egui::pos2(48.0, 48.0), egui::vec2(1424.0, 729.0), laptop)), None);
        // Never smaller than the minimum size, and never off the top-left corner.
        let tiny = egui::vec2(800.0, 600.0);
        assert_eq!(system.fit_on_monitor(&opened(at, start, tiny)), Some((egui::pos2(12.0, 40.5), MIN_SIZE)));
        // Maximized, full screen or minimized, or the monitor not known yet: nothing to do.
        let mut i = opened(at, egui::vec2(1440.0, 864.0), laptop);
        assert_eq!(WindowGeometry { maximized: true, ..saved() }.fit_on_monitor(&i), None);
        i.maximized = Some(true);
        assert_eq!(system.fit_on_monitor(&i), None);
        i.maximized = None;
        i.fullscreen = Some(true);
        assert_eq!(system.fit_on_monitor(&i), None);
        i.fullscreen = None;
        i.minimized = Some(true);
        assert_eq!(system.fit_on_monitor(&i), None);
        i.minimized = None;
        i.monitor_size = None;
        assert_eq!(system.fit_on_monitor(&i), None);
        // Wayland reports no rects.
        assert_eq!(system.fit_on_monitor(&RawWindowState { monitor_size: Some(laptop), ..Default::default() }), None);
    }

    fn r(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
    }

    /// The 400 × 200 dialog window `content` centres.
    fn centred(content: egui::Rect) -> egui::Rect {
        egui::Rect::from_center_size(content.center(), egui::vec2(400.0, 200.0))
    }

    /// #4842: a dialog on screen stays put; one off screen is centred on the part of the window
    /// on screen, or brings a window too far off screen back just far enough to show it.
    #[test]
    fn a_dialog_off_screen_comes_onto_the_visible_part_of_its_window() {
        let primary = r(0.0, 0.0, 1920.0, 1080.0);
        let left = r(-1920.0, 0.0, 1920.0, 1080.0);
        let above = r(0.0, -1440.0, 2560.0, 1440.0);
        let place = |content: egui::Rect, monitors: &[egui::Rect]| place_dialog_on_screen(content, monitors, centred(content));
        // On screen: nothing changes, also on a monitor left of or above the primary one, and
        // for a window across two side-by-side monitors.
        assert_eq!(place(r(100.0, 100.0, 1440.0, 900.0), &[primary]), None);
        assert_eq!(place(r(-1800.0, 100.0, 1440.0, 900.0), &[left, primary]), None);
        assert_eq!(place(r(100.0, -1300.0, 1440.0, 900.0), &[primary, above]), None);
        assert_eq!(place(r(-700.0, 100.0, 1440.0, 900.0), &[left, primary]), None);
        // Off the left edge with 740 showing: centred on those, across only.
        let p = place(r(-700.0, 100.0, 1440.0, 900.0), &[primary]);
        assert_eq!(p, Some(DialogPlacement { dialog_shift: egui::vec2(350.0, 0.0), window_shift: egui::Vec2::ZERO }));
        // Off the bottom-right corner: centred on the corner that shows.
        let content = r(1200.0, 700.0, 1440.0, 900.0);
        let Some(p) = place(content, &[primary]) else { panic!("moved") };
        let at = centred(content).translate(p.dialog_shift);
        assert_eq!(at, egui::Rect::from_center_size(egui::pos2(1560.0, 890.0), egui::vec2(400.0, 200.0)));
        assert_eq!(p.window_shift, egui::Vec2::ZERO);
        // 40 points showing: the dialog takes the window's right edge, and the window moves
        // 360 points right to show it.
        let content = r(-1400.0, 100.0, 1440.0, 900.0);
        let Some(p) = place(content, &[primary]) else { panic!("moved") };
        let at = centred(content).translate(p.dialog_shift);
        assert_eq!((at.left(), at.right()), (-360.0, 40.0));
        assert_eq!(p.window_shift, egui::vec2(360.0, 0.0));
        assert!(primary.contains_rect(at.translate(p.window_shift)));
        // Entirely off screen, right of the right-most monitor: onto the nearest one.
        let content = r(5000.0, 100.0, 1440.0, 900.0);
        let Some(p) = place(content, &[left, primary]) else { panic!("moved") };
        let at = centred(content).translate(p.dialog_shift + p.window_shift);
        assert_eq!((at.left(), at.right()), (1520.0, 1920.0));
        // Above every monitor: down onto the nearest one.
        let content = r(100.0, -3000.0, 1440.0, 900.0);
        let Some(p) = place(content, &[primary]) else { panic!("moved") };
        let at = centred(content).translate(p.dialog_shift + p.window_shift);
        assert!(primary.contains_rect(at), "{at:?}");
        // A dialog wider than its window shows its start.
        let small = r(-100.0, 0.0, 300.0, 300.0);
        let Some(p) = place(small, &[primary]) else { panic!("moved") };
        assert_eq!(centred(small).translate(p.dialog_shift + p.window_shift).left(), 0.0);
    }

    /// Unknown or insane geometry changes nothing and never panics.
    #[test]
    fn hostile_geometry_leaves_dialogs_alone() {
        let primary = r(0.0, 0.0, 1920.0, 1080.0);
        let off = r(-1400.0, 100.0, 1440.0, 900.0);
        let bad = [
            r(f32::NAN, 0.0, 100.0, 100.0),
            r(0.0, 0.0, f32::INFINITY, 100.0),
            r(f32::NEG_INFINITY, 0.0, 100.0, 100.0),
            r(0.0, 0.0, 0.0, 100.0),
            r(0.0, 0.0, 100.0, -5.0),
            r(1e30, 1e30, 100.0, 100.0),
            egui::Rect::NOTHING,
            egui::Rect::EVERYTHING,
        ];
        for b in bad {
            assert_eq!(place_dialog_on_screen(b, &[primary], centred(off)), None, "content {b:?}");
            assert_eq!(place_dialog_on_screen(off, &[primary], b), None, "dialog {b:?}");
            // An insane monitor is ignored: with none left, nothing moves.
            assert_eq!(place_dialog_on_screen(off, &[b], centred(off)), None, "monitor {b:?}");
            assert!(place_dialog_on_screen(off, &[b, primary], centred(off)).is_some(), "the sane monitor counts: {b:?}");
        }
        assert_eq!(place_dialog_on_screen(off, &[], centred(off)), None, "no monitors");
        // Tiny but valid rects give finite answers.
        let p = place_dialog_on_screen(r(-0.25, 0.0, 0.5, 0.5), &[r(0.0, 0.0, 0.1, 0.1)], r(-1.0, -1.0, 2.0, 2.0));
        assert!(p.is_none_or(|p| p.dialog_shift.is_finite() && p.window_shift.is_finite()), "{p:?}");
    }

    /// The host's monitors are kept in pixels and read in the viewport's points; none set, none known.
    #[test]
    fn monitors_are_kept_in_pixels_and_read_in_points() {
        let ctx = egui::Context::default();
        assert!(monitors_in_points(&ctx).is_empty());
        set_monitors(&ctx, vec![r(-3840.0, 0.0, 3840.0, 2160.0)]);
        let mut raw = egui::RawInput::default();
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(2.0);
        ctx.begin_pass(raw);
        let monitors = monitors_in_points(&ctx);
        ctx.end_pass().drop_without_applying_deltas();
        assert_eq!(monitors, vec![r(-1920.0, 0.0, 1920.0, 1080.0)]);
    }
}

//! The window size limits: the smallest window the desktop app allows and the largest size a
//! caller may ask a window for.
//!
//! Sizes are in egui points.

<<<<<<< HEAD
/// The smallest window: the desktop app's minimum inner size.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(760.0, 480.0);
/// Larger requested sizes are refused. eframe also shrinks the start size to the largest monitor.
=======
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
    /// The host's [`reachable`] answer: `None` where it can't list the monitors (Wayland).
    pub on_a_monitor: Option<bool>,
    /// The primary monitor's bounds in egui points, when the host can identify it.
    pub primary_monitor_rect: Option<egui::Rect>,
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
            on_a_monitor: None,
            primary_monitor_rect: None,
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

/// The desktop app's minimum inner size, and the smallest window restored. 600 pt wide (#5886):
/// allows half-screen split window tiling on standard displays (e.g. 1280 pt or 1366 pt, such as
/// under GNOME Wayland), and the widest dialogs that fitted 760 pt (Organizer, Define New
/// Multilevel List) still fit.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(600.0, 480.0);
/// Larger saved sizes are corrupt. eframe also shrinks the start size to the largest monitor.
>>>>>>> up/main
const MAX_SIZE: egui::Vec2 = egui::vec2(16384.0, 16384.0);

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

#[cfg(test)]
mod tests {
    use super::*;

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
<<<<<<< HEAD
=======

    /// #5886: window can be resized to half-screen split widths on standard screens (1280 pt, 1366 pt, etc.)
    /// down to the exact 600 pt minimum, including 1080p displays with 125% and 150% scaling.
    /// Clamping at 760 prevented Wayland and GNOME window managers from tiling the window side-by-side.
    #[test]
    fn requested_size_allows_split_window_narrow_widths() {
        // Exact 600 pt minimum width.
        assert_eq!(requested_size(MIN_SIZE, 8192.0), Some(MIN_SIZE));
        assert_eq!(requested_size(egui::vec2(600.0, 480.0), 8192.0), Some(egui::vec2(600.0, 480.0)));

        // 1080p display at 150% scaling (1920 / 1.5 = 1280 pt total width -> 640 pt half-screen split).
        // Also 1280x720 / 1280x800 at 100% scaling (1280 / 2 = 640 pt).
        assert_eq!(requested_size(egui::vec2(640.0, 480.0), 8192.0), Some(egui::vec2(640.0, 480.0)));
        assert_eq!(requested_size(egui::vec2(640.0, 540.0), 8192.0), Some(egui::vec2(640.0, 540.0)));

        // 1366x768 display at 100% scaling (1366 / 2 = 683 pt half-screen split).
        assert_eq!(requested_size(egui::vec2(683.0, 480.0), 8192.0), Some(egui::vec2(683.0, 480.0)));

        // 1080p display at 125% scaling (1920 / 1.25 = 1536 pt total width -> 768 pt half-screen split).
        assert_eq!(requested_size(egui::vec2(768.0, 480.0), 8192.0), Some(egui::vec2(768.0, 480.0)));

        // Scales above 150% (e.g. 175% where half width is ~548.5 pt) clamp up to the 600 pt minimum width.
        assert_eq!(requested_size(egui::vec2(548.0, 480.0), 8192.0), Some(egui::vec2(600.0, 480.0)));

        // Sanitized geometry preserves these split-window sizes.
        let geom = WindowGeometry { position: Some([0.0, 0.0]), width: 640.0, height: 480.0, maximized: false, monitor: None };
        assert_eq!(geom.sanitized().map(|g| (g.width, g.height)), Some((640.0, 480.0)));
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
        let tiny = egui::vec2(MIN_SIZE.x + 40.0, 600.0);
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

    /// On displays with smaller dimensions or high DPI scaling (e.g. 2560x1440 at 200% scale,
    /// yielding a 1280x720 pt screen), the requested inner size shrinks to fit the screen
    /// rather than extending past the boundaries (#5871).
    #[test]
    fn fit_inner_size_constrains_window_to_available_screen_dimensions() {
        let default_inner = egui::vec2(1440.0, 920.0);
        let frame = egui::vec2(16.0, 39.0);

        // 2560x1440 display with 200% scaling: 1280x720 pt.
        let scaled_1440p = egui::vec2(1280.0, 720.0);
        let fitted = fit_inner_size(default_inner, scaled_1440p, frame);
        assert_eq!(fitted, egui::vec2(1168.0, 585.0));
        assert!(fitted.x + frame.x <= scaled_1440p.x);
        assert!(fitted.y + frame.y <= scaled_1440p.y);

        // Standard 2560x1440 at 100% scaling: ample room, size remains untouched.
        let large_monitor = egui::vec2(2560.0, 1440.0);
        assert_eq!(fit_inner_size(default_inner, large_monitor, frame), default_inner);

        // Tiny display: clamps to minimum inner size.
        let tiny = egui::vec2(600.0, 400.0);
        assert_eq!(fit_inner_size(default_inner, tiny, frame), MIN_SIZE);

        // Hostile / non-finite inputs never panic and fall back safely.
        assert_eq!(fit_inner_size(egui::vec2(f32::NAN, 100.0), scaled_1440p, frame), MIN_SIZE);
        assert_eq!(fit_inner_size(default_inner, egui::vec2(f32::INFINITY, f32::NAN), frame), default_inner.max(MIN_SIZE));

        // Viewport builder adaptation preserves other builder properties while clamping inner size.
        let builder = egui::ViewportBuilder::default().with_inner_size([1440.0, 920.0]).with_title("Test");
        let adapted = fit_builder_to_monitor(builder, scaled_1440p, frame);
        assert_eq!(adapted.inner_size, Some(egui::vec2(1168.0, 585.0)));
        assert_eq!(adapted.title, Some("Test".into()));
    }
>>>>>>> up/main
}

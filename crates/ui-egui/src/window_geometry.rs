//! The main window's size, position and maximized state, saved with the UI preferences so the
//! next launch opens the window where the user left it (#23).
//!
//! Sizes and positions are in egui points: what [`egui::ViewportInfo`] reports and what
//! [`egui::ViewportBuilder`] takes.

use serde::{Deserialize, Serialize};

/// The smallest window restored: the desktop app's minimum inner size.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(760.0, 480.0);
/// Larger saved sizes are corrupt. eframe also shrinks the start size to the largest monitor.
const MAX_SIZE: egui::Vec2 = egui::vec2(16384.0, 16384.0);
/// Saved positions farther from the origin than this are corrupt.
const MAX_COORD: f32 = 32768.0;

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
    pub fn track(prev: Option<Self>, info: &egui::ViewportInfo, content_size: egui::Vec2) -> Option<Self> {
        if info.minimized == Some(true) || info.fullscreen == Some(true) {
            return prev;
        }
        // egui reads the maximized state on macOS only when the window opens (egui#3494), so a
        // window that opened zoomed would stay "maximized" for good. There the zoomed frame is
        // saved like any other size instead.
        let maximized = info.maximized == Some(true) && !cfg!(target_os = "macos");
        if maximized && let Some(prev) = prev {
            return Some(Self { maximized, ..prev });
        }
        let rect = if cfg!(target_os = "macos") { info.inner_rect } else { info.outer_rect };
        let size = info.inner_rect.map_or(content_size, |r| r.size());
        let position = rect.map(|r| [r.min.x, r.min.y]);
        let monitor = info.monitor_size.map(|s| [s.x, s.y]);
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

    /// Open the window at this size, position and maximized state.
    pub fn apply(self, builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
        let builder = builder.with_inner_size([self.width, self.height]).with_maximized(self.maximized);
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
    pub fn rescue_position(&self, info: &egui::ViewportInfo) -> Option<egui::Pos2> {
        if self.maximized || self.position.is_none() {
            return None;
        }
        let saved = egui::Vec2::from(self.monitor?);
        let now = info.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)?;
        if (now - saved).length() <= 1.0 {
            return None;
        }
        let free = now - egui::vec2(self.width, self.height);
        Some(egui::pos2((free.x / 2.0).max(0.0), (free.y / 2.0).max(0.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(rect: egui::Rect, monitor: egui::Vec2) -> egui::ViewportInfo {
        egui::ViewportInfo { inner_rect: Some(rect), outer_rect: Some(rect), monitor_size: Some(monitor), ..Default::default() }
    }

    fn saved() -> WindowGeometry {
        WindowGeometry { position: Some([100.0, 50.0]), width: 1200.0, height: 800.0, maximized: false, monitor: Some([1920.0, 1080.0]) }
    }

    #[test]
    fn records_the_window_and_restores_it() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1200.0, 800.0));
        let g = WindowGeometry::track(None, &info(rect, egui::vec2(1920.0, 1080.0)), egui::Vec2::ZERO);
        assert_eq!(g, Some(saved()));
        let b = saved().apply(egui::ViewportBuilder::default());
        assert_eq!(b.inner_size, Some(egui::vec2(1200.0, 800.0)));
        assert_eq!(b.position, Some(egui::pos2(100.0, 50.0)));
        assert_eq!(b.maximized, Some(false));
    }

    #[test]
    fn maximized_keeps_the_normal_size_and_position() {
        let full = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1040.0));
        let mut i = info(full, egui::vec2(1920.0, 1080.0));
        i.maximized = Some(true);
        let g = WindowGeometry::track(Some(saved()), &i, egui::Vec2::ZERO);
        if cfg!(target_os = "macos") {
            // The zoomed frame is saved as the size; the window never reopens "maximized".
            assert_eq!(g.map(|g| (g.position, g.width, g.height, g.maximized)), Some((Some([0.0, 0.0]), 1920.0, 1040.0, false)));
        } else {
            assert_eq!(g, Some(WindowGeometry { maximized: true, ..saved() }));
        }
        // Minimized or full screen: nothing changes.
        i.maximized = Some(false);
        i.minimized = Some(true);
        assert_eq!(WindowGeometry::track(Some(saved()), &i, egui::Vec2::ZERO), Some(saved()));
        i.minimized = None;
        i.fullscreen = Some(true);
        assert_eq!(WindowGeometry::track(Some(saved()), &i, egui::Vec2::ZERO), Some(saved()));
    }

    #[test]
    fn without_a_position_the_content_size_is_kept() {
        // Wayland reports neither rect.
        let i = egui::ViewportInfo::default();
        let g = WindowGeometry::track(None, &i, egui::vec2(1000.0, 700.0));
        assert_eq!(g, Some(WindowGeometry { position: None, width: 1000.0, height: 700.0, maximized: false, monitor: None }));
        assert_eq!(g.map(|g| g.apply(egui::ViewportBuilder::default()).position), Some(None));
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

    #[test]
    fn a_window_whose_monitor_is_gone_is_moved_on_screen() {
        let on = egui::Rect::from_min_size(egui::pos2(2000.0, 50.0), egui::vec2(1200.0, 800.0));
        // Same monitor: stays where it was.
        assert_eq!(saved().rescue_position(&info(on, egui::vec2(1920.0, 1080.0))), None);
        // A different (smaller) monitor now: centred on it.
        assert_eq!(saved().rescue_position(&info(on, egui::vec2(1600.0, 900.0))), Some(egui::pos2(200.0, 50.0)));
        // Larger than that monitor: its top-left corner.
        assert_eq!(saved().rescue_position(&info(on, egui::vec2(1024.0, 768.0))), Some(egui::Pos2::ZERO));
        // Nothing to compare, or no saved position: no move.
        assert_eq!(WindowGeometry { monitor: None, ..saved() }.rescue_position(&info(on, egui::vec2(1600.0, 900.0))), None);
        assert_eq!(WindowGeometry { position: None, ..saved() }.rescue_position(&info(on, egui::vec2(1600.0, 900.0))), None);
        assert_eq!(WindowGeometry { maximized: true, ..saved() }.rescue_position(&info(on, egui::vec2(1600.0, 900.0))), None);
        assert_eq!(saved().rescue_position(&egui::ViewportInfo::default()), None);
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
}

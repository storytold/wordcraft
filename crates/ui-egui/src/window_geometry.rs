//! The window size limits: the smallest window the desktop app allows and the largest size a
//! caller may ask a window for.
//!
//! Sizes are in egui points.

/// The smallest window: the desktop app's minimum inner size. 600 pt wide (#5886): allows
/// half-screen split window tiling on standard displays (e.g. 1280 pt or 1366 pt, such as under
/// GNOME Wayland), and the widest dialogs that fitted 760 pt (Organizer, Define New Multilevel
/// List) still fit.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(600.0, 480.0);
/// Larger requested sizes are refused. eframe also shrinks the start size to the largest monitor.
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
    }

    /// #7317: insane values in requested window geometry are clamped or dropped.
    #[test]
    fn insane_values_are_clamped_or_dropped() {
        // Zero or non-positive width/height are dropped.
        assert_eq!(requested_size(egui::vec2(0.0, 800.0), 16384.0), None);
        assert_eq!(requested_size(egui::vec2(800.0, 0.0), 16384.0), None);
        assert_eq!(requested_size(egui::vec2(-5.0, 800.0), 16384.0), None);

        // NaN and infinite values are dropped.
        assert_eq!(requested_size(egui::vec2(f32::NAN, 800.0), 16384.0), None);
        assert_eq!(requested_size(egui::vec2(800.0, f32::NAN), 16384.0), None);
        assert_eq!(requested_size(egui::vec2(f32::INFINITY, 800.0), 16384.0), None);
        assert_eq!(requested_size(egui::vec2(800.0, f32::NEG_INFINITY), 16384.0), None);

        // Tiny side clamps up to MIN_SIZE, huge side clamps down to MAX_SIZE (16384.0).
        let tiny_and_huge = requested_size(egui::vec2(10.0, 1.0e9), 16384.0);
        assert_eq!(tiny_and_huge, Some(egui::vec2(MIN_SIZE.x, 16384.0)));

        // Sane values within bounds are preserved as-is.
        let sane = egui::vec2(1200.0, 800.0);
        assert_eq!(requested_size(sane, 16384.0), Some(sane));
    }
}

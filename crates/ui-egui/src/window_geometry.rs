//! The window size limits: the smallest window the desktop app allows and the largest size a
//! caller may ask a window for.
//!
//! Sizes are in egui points.

/// The smallest window: the desktop app's minimum inner size.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(760.0, 480.0);
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
}

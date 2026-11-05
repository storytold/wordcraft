//! Middle-click scrolling of the document canvas (#1065), the same gesture as Word on Windows
//! and pdfcraft's viewer: click the wheel and a marker shows where; moving the pointer above or
//! below it scrolls, faster the farther away. Another click, Escape, any other key, the wheel or
//! leaving the window stops it. Holding the wheel down and moving scrolls too, and then letting
//! go stops.
//!
//! WordCraft doesn't paste the primary selection on a middle click, so on Linux the button is
//! free for this. macOS apps don't scroll this way, and browsers have their own middle-click
//! behaviour, so there the button is left alone.

use egui::{CursorIcon, Event, Key, PointerButton, Pos2, Stroke, Vec2, vec2};

/// Where the gesture exists at all (see the module docs).
const SUPPORTED: bool = !cfg!(any(target_os = "macos", target_os = "ios", target_arch = "wasm32"));

/// Pointer distance (screen points) around the anchor that doesn't scroll.
const DEAD_ZONE: f32 = 15.0;
// Chromium's autoscroll_controller.cc uses distance^2.2 * 0.000008; its
// ui/events/gestures/fixed_velocity_curve.cc multiplies elapsed seconds by 5000.
const SPEED_EXPONENT: f32 = 2.2;
const SPEED_MULTIPLIER: f32 = 0.04;
// Bound hostile coordinates before exponentiation, far beyond ordinary screen distances.
const MAX_DISPLACEMENT: f32 = 1_000_000.0;

#[derive(Debug)]
pub(crate) struct AutoScroll {
    /// Where the wheel was clicked; `Some` while scrolling.
    anchor: Option<Pos2>,
    /// The press that started it is still down…
    holding: bool,
    /// …and the pointer has left the dead zone since: letting go ends it.
    dragged: bool,
    /// Own a stopping click through its release, so it can't also move the caret.
    cancel_button: Option<PointerButton>,
    block_input: bool,
    pub(crate) supported: bool,
}

impl Default for AutoScroll {
    fn default() -> Self {
        Self { anchor: None, holding: false, dragged: false, cancel_button: None, block_input: false, supported: SUPPORTED }
    }
}

impl AutoScroll {
    pub(crate) fn active(&self) -> bool {
        self.anchor.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.anchor = None;
        self.holding = false;
        self.dragged = false;
        self.cancel_button = None;
        self.block_input = false;
    }

    /// Mouse buttons belong to the gesture this frame: the canvas mustn't act on them.
    pub(crate) fn blocks_input(&self) -> bool {
        self.block_input
    }

    /// Run before the canvas scrolls. Returns this frame's delta (positive moves the content
    /// down, as the canvas's wheel delta). Starting is restricted to the visible canvas; once
    /// started, moving outside it still controls the speed.
    pub(crate) fn update(&mut self, ui: &egui::Ui, viewport: egui::Rect) -> Vec2 {
        if !self.supported {
            self.cancel();
            return Vec2::ZERO;
        }
        let ctx = ui.ctx();
        let (pointer, middle_press, middle_down, middle_released, cancel, cancel_button, dt) = ctx.input(|i| {
            let cancel_button = [PointerButton::Primary, PointerButton::Secondary, PointerButton::Extra1, PointerButton::Extra2]
                .into_iter()
                .find(|button| i.pointer.button_pressed(*button));
            (
                i.pointer.hover_pos(),
                // The press event itself: later movement in this frame mustn't move the anchor.
                i.events.iter().find_map(|event| match event {
                    Event::PointerButton { pos, button: PointerButton::Middle, pressed: true, .. } if pos.is_finite() => Some(*pos),
                    _ => None,
                }),
                i.pointer.button_down(PointerButton::Middle),
                i.pointer.button_released(PointerButton::Middle),
                !i.focused
                    || cancel_button.is_some()
                    || i.events.iter().any(|e| matches!(e, Event::Key { pressed: true, .. } | Event::MouseWheel { .. } | Event::Zoom(_))),
                cancel_button,
                i.stable_dt,
            )
        });
        self.block_input = self.active() || self.cancel_button.is_some() || middle_press.is_some() || middle_down || middle_released;
        if let Some(button) = self.cancel_button {
            if !ctx.input(|i| i.pointer.button_down(button)) {
                self.cancel_button = None;
            }
            return Vec2::ZERO;
        }
        if self.active() && (cancel || pointer.is_none()) {
            // Escape only stops scrolling; any other key stops it and does its usual job.
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
            self.cancel();
            self.block_input = true;
            self.cancel_button = cancel_button;
            return Vec2::ZERO;
        }
        if let Some(pressed_at) = middle_press {
            if self.active() {
                self.cancel();
                self.block_input = true;
                self.cancel_button = Some(PointerButton::Middle);
                return Vec2::ZERO;
            } else if !cancel && viewport.intersect(ui.clip_rect()).contains(pressed_at) && ctx.layer_id_at(pressed_at) == Some(ui.layer_id()) {
                self.anchor = Some(pressed_at);
                self.holding = true;
                self.dragged = false;
            }
        }
        let (Some(anchor), Some(pointer)) = (self.anchor, pointer) else { return Vec2::ZERO };
        let displacement = pointer.y - anchor.y;
        if self.holding {
            self.dragged |= displacement.abs() > DEAD_ZONE;
            if !middle_down || middle_released {
                self.holding = false;
                if self.dragged {
                    self.cancel();
                    self.block_input = true;
                    return Vec2::ZERO;
                }
            }
        }
        if displacement.is_finite() && displacement.abs() > DEAD_ZONE {
            // Continuous redraws let stable_dt use measured frame time, so the speed doesn't
            // depend on the frame rate.
            ctx.request_repaint();
        }
        vec2(0.0, scroll_delta(displacement, dt))
    }

    /// Draw a marker at the anchor, above the pages, and the cursor. Run after the
    /// canvas has set its own cursor.
    pub(crate) fn paint(&self, ui: &egui::Ui, viewport: egui::Rect) {
        let Some(anchor) = self.anchor else { return };
        let painter = ui.painter().with_clip_rect(viewport);
        let ink = ui.visuals().text_color();
        painter.circle(anchor, 13.0, ui.visuals().window_fill(), Stroke::new(1.0, ink));
        painter.circle_filled(anchor, 2.0, ink);
        for direction in [-1.0, 1.0] {
            painter.add(egui::Shape::convex_polygon(
                vec![anchor + vec2(-4.0, direction * 6.0), anchor + vec2(4.0, direction * 6.0), anchor + vec2(0.0, direction * 10.0)],
                ink,
                Stroke::NONE,
            ));
        }
        // No pointer position (outside the window) counts as the dead zone: `update` cancels the gesture then.
        let displacement = ui.ctx().input(|i| i.pointer.hover_pos()).map_or(0.0, |p| p.y - anchor.y);
        ui.ctx().set_cursor_icon(cursor_icon(displacement));
    }
}

fn scroll_delta(displacement: f32, dt: f32) -> f32 {
    if !displacement.is_finite() || !dt.is_finite() {
        return 0.0;
    }
    let distance = displacement.abs();
    if distance <= DEAD_ZONE {
        return 0.0;
    }
    let speed = distance.min(MAX_DISPLACEMENT).powf(SPEED_EXPONENT) * SPEED_MULTIPLIER;
    // Pointer below the anchor scrolls down: the content moves up. A stalled frame (an idle or
    // hidden window) mustn't jump pages.
    -displacement.signum() * speed * dt.clamp(0.0, 0.05)
}

/// The cursor for a pointer `displacement` points (positive = below) from the anchor: the
/// up-down arrow while it rests in the dead zone, otherwise an arrow towards the scroll direction.
fn cursor_icon(displacement: f32) -> CursorIcon {
    if !displacement.is_finite() || displacement.abs() <= DEAD_ZONE {
        CursorIcon::ResizeVertical
    } else if displacement < 0.0 {
        CursorIcon::ResizeNorth
    } else {
        CursorIcon::ResizeSouth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_has_a_dead_zone_grows_with_distance_and_rejects_invalid_input() {
        for y in [-15.0, -1.0, 0.0, 1.0, 15.0] {
            assert_eq!(scroll_delta(y, 0.016), 0.0);
        }
        assert!(scroll_delta(30.0, 0.016) < 0.0, "below the anchor scrolls down");
        assert_eq!(scroll_delta(30.0, 0.016), -scroll_delta(-30.0, 0.016));
        for (near, far) in [(16.0, 30.0), (30.0, 100.0), (100.0, 200.0)] {
            assert!(scroll_delta(far, 0.016).abs() > scroll_delta(near, 0.016).abs());
        }
        assert_eq!(scroll_delta(1000.0, 10.0), scroll_delta(1000.0, 0.05));
        assert_eq!(scroll_delta(1000.0, -1.0), 0.0);
        assert!(scroll_delta(f32::MAX, 0.016).is_finite());
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(scroll_delta(invalid, 0.016), 0.0);
            assert_eq!(scroll_delta(1000.0, invalid), 0.0);
        }
    }

    #[test]
    fn cursor_follows_the_scroll_direction_outside_the_dead_zone() {
        for y in [-15.0, -1.0, 0.0, 1.0, 15.0] {
            assert_eq!(cursor_icon(y), CursorIcon::ResizeVertical);
        }
        assert_eq!(cursor_icon(-16.0), CursorIcon::ResizeNorth);
        assert_eq!(cursor_icon(16.0), CursorIcon::ResizeSouth);
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(cursor_icon(invalid), CursorIcon::ResizeVertical);
        }
    }
}

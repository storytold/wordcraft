//! PdfCraft patch (13): Wayland scroll-phase policy, shared with the regression tests
//! (`tests/pointer_patches.rs`).

use wayland_client::protocol::wl_pointer::AxisSource;

use crate::event::TouchPhase;

pub(crate) fn phase(
    previous: TouchPhase,
    source: Option<AxisSource>,
    has_discrete_scroll: bool,
    stopped: bool,
) -> TouchPhase {
    // Wayland does not guarantee axis_stop for these sources. A pixel-only wheel must
    // not start a finger gesture: consumers would retain its modifiers indefinitely.
    // https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_pointer-event-axis_source
    if stopped {
        TouchPhase::Ended
    } else if has_discrete_scroll
        || matches!(
            source,
            Some(AxisSource::Wheel | AxisSource::WheelTilt | AxisSource::Continuous)
        )
    {
        TouchPhase::Moved
    } else {
        match previous {
            TouchPhase::Started | TouchPhase::Moved => TouchPhase::Moved,
            _ => TouchPhase::Started,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheels_never_start_a_gesture() {
        for source in [AxisSource::Wheel, AxisSource::WheelTilt, AxisSource::Continuous] {
            for previous in [TouchPhase::Ended, TouchPhase::Started, TouchPhase::Moved] {
                assert_eq!(phase(previous, Some(source), false, false), TouchPhase::Moved);
            }
        }
        assert_eq!(
            phase(TouchPhase::Ended, Some(AxisSource::Wheel), true, false),
            TouchPhase::Moved
        );
    }

    #[test]
    fn finger_gestures_start_and_end() {
        let finger = Some(AxisSource::Finger);
        assert_eq!(phase(TouchPhase::Ended, finger, false, false), TouchPhase::Started);
        assert_eq!(phase(TouchPhase::Started, finger, false, false), TouchPhase::Moved);
        assert_eq!(phase(TouchPhase::Moved, finger, false, true), TouchPhase::Ended);
    }

    #[test]
    fn unknown_sources_keep_the_previous_policy() {
        assert_eq!(phase(TouchPhase::Ended, None, false, false), TouchPhase::Started);
        assert_eq!(phase(TouchPhase::Started, None, false, false), TouchPhase::Moved);
        assert_eq!(phase(TouchPhase::Ended, None, true, false), TouchPhase::Moved);
        assert_eq!(phase(TouchPhase::Moved, None, false, true), TouchPhase::Ended);
    }
}

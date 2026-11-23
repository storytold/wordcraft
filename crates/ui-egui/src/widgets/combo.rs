//! Combo box geometry and outline helpers.

use egui::{CornerRadius, Rect, pos2};

const ARROW_BORDER: f32 = 1.0;

fn arrow_box_shape(rtl: bool, arrow: Rect, r: CornerRadius) -> (CornerRadius, Rect) {
    if rtl {
        (CornerRadius { nw: r.nw, sw: r.sw, ne: 0, se: 0 }, Rect::from_min_max(arrow.min, pos2(arrow.max.x + ARROW_BORDER, arrow.max.y)))
    } else {
        (CornerRadius { nw: 0, sw: 0, ne: r.ne, se: r.se }, Rect::from_min_max(pos2(arrow.min.x - ARROW_BORDER, arrow.min.y), arrow.max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arrow's box fuses with its field on the side they meet: in a left-to-right row the
    /// arrow is right of the field (rounded on the right, painted one border under the field's
    /// right edge), in a right-to-left row it is left of it (rounded on the left, under the
    /// field's left edge).
    #[test]
    fn the_arrow_box_is_rounded_away_from_its_field() {
        let arrow = Rect::from_min_max(pos2(100.0, 0.0), pos2(116.0, 20.0));
        let r = CornerRadius::same(4);
        let (ltr, under) = arrow_box_shape(false, arrow, r);
        assert_eq!((ltr.nw, ltr.sw, ltr.ne, ltr.se), (0, 0, 4, 4));
        assert_eq!((under.min.x, under.max.x), (100.0 - ARROW_BORDER, 116.0));
        let (rtl, under) = arrow_box_shape(true, arrow, r);
        assert_eq!((rtl.nw, rtl.sw, rtl.ne, rtl.se), (4, 4, 0, 0));
        assert_eq!((under.min.x, under.max.x), (100.0, 116.0 + ARROW_BORDER));
    }
}

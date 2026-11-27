//! Imported drawing frames and their separately retained picture geometry.

use wordcraft_doc::para::{Float, InlineObject};
use wordcraft_geom::Rect;

/// The picture's transform fits uniformly, aligned to the frame's top left.
/// Group members have no separate outer extent and keep their group's scale.
pub(crate) fn picture_rect(object: &InlineObject, rect: Rect) -> Rect {
    let InlineObject::Image { w, h, float, .. } = object else { return rect };
    let (w, h) = float.picture_extent.map_or((*w, *h), |[w, h]| (w as f32, h as f32));
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return rect;
    }
    let scale = (rect.w / w).min(rect.h / h);
    Rect::new(rect.x, rect.y, w * scale, h * scale)
}

/// Word imports outer picture and group boxes in twips. The model keeps their original
/// point values so layout quantization cannot change a later DOCX save.
pub(crate) fn frame(object: &InlineObject) -> Option<(f32, f32, &Float)> {
    object.frame().map(|(w, h, float)| {
        if matches!(object, InlineObject::Image { .. } | InlineObject::Group { .. }) {
            let twips = |v: f32| (wordcraft_geom::finite(v).clamp(0.0, 4000.0) * 20.0).trunc() / 20.0;
            (twips(w), twips(h), float)
        } else {
            (w, h, float)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_pictures_fit_the_transform_without_changing_the_saved_frame() {
        let object = InlineObject::Image {
            media: String::new(),
            w: 120.0,
            h: 60.12,
            alt: String::new(),
            crop: [0.0; 4],
            float: Float { picture_extent: Some([1_270_000, 889_000]), ..Default::default() },
            fx: Default::default(),
            ole: None,
        };
        let (w, h, _) = frame(&object).unwrap();
        let original = object.frame().unwrap();
        assert!(h < original.1 && original.1 - h < 0.05);
        let outer = Rect::new(80.0, 100.0, w, h);
        let picture = picture_rect(&object, outer);
        assert_eq!((picture.x, picture.y), (outer.x, outer.y));
        assert!(picture.w < outer.w && (picture.h - outer.h).abs() < 0.001);
        assert!((picture.w / picture.h - 100.0 / 70.0).abs() < 0.001);
        assert_eq!((original.0, original.1), (120.0, 60.12), "layout never changes the saved box");
        for wrap in [wordcraft_doc::para::Wrap::Inline, wordcraft_doc::para::Wrap::InFrontOfText] {
            let mut d = wordcraft_doc::Document::from_text("");
            let mut image = object.clone();
            image.float_mut().unwrap().wrap = wrap;
            d.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().insert_object(0, image, &Default::default()).unwrap();
            let laid = crate::layout(&d, &mut crate::LayoutCache::new(), &Default::default());
            let display = crate::display::page_display(&d, &laid.pages[0], &Default::default());
            let drawn = display
                .iter()
                .find_map(|draw| match draw {
                    crate::display::Draw::Image { rect, .. } => Some(*rect),
                    _ => None,
                })
                .unwrap();
            assert!((drawn.w - picture.w).abs() < 0.001 && (drawn.h - picture.h).abs() < 0.001, "{wrap:?}: {drawn:?}");
        }
    }

    #[test]
    fn group_members_keep_fractional_sizes_and_anisotropic_scaling() {
        let child = InlineObject::Image {
            media: String::new(),
            w: 120.0,
            h: 60.13,
            alt: String::new(),
            crop: [0.0; 4],
            float: Default::default(),
            fx: Default::default(),
            ole: None,
        };
        for wrap in [wordcraft_doc::para::Wrap::Inline, wordcraft_doc::para::Wrap::InFrontOfText] {
            let group = InlineObject::Group {
                w: 240.0,
                h: 180.24,
                ch_w: 120.0,
                ch_h: 60.12,
                float: Float { wrap, ..Default::default() },
                canvas: None,
                children: vec![wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: child.clone() }],
            };
            let (_, outer_h, _) = frame(&group).unwrap();
            let mut d = wordcraft_doc::Document::from_text("");
            d.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().insert_object(0, group, &Default::default()).unwrap();
            let laid = crate::layout(&d, &mut crate::LayoutCache::new(), &Default::default());
            let display = crate::display::page_display(&d, &laid.pages[0], &Default::default());
            let drawn = display
                .iter()
                .find_map(|draw| match draw {
                    crate::display::Draw::Image { rect, .. } => Some(*rect),
                    _ => None,
                })
                .unwrap();
            assert!((drawn.w - 240.0).abs() < 0.001, "the group retains its independent horizontal scale");
            assert!(drawn.h > outer_h, "the slightly taller child must overhang the group's frame");
            assert!((drawn.h / outer_h - 60.13 / 60.12).abs() < 0.00001, "fractional child and coordinate-space heights survive scaling");
        }
    }
}

use super::{DisplayOptions, Draw, Rgb, Session, document_with_ink, page_display};
use wordcraft_doc::graphic::PathSeg;

fn ink_width(draws: &[Draw]) -> Option<f32> {
    draws.iter().find_map(|draw| match draw {
        Draw::NativeInk { width, .. } => Some(*width),
        Draw::Figure { draws, .. } => ink_width(draws),
        Draw::Rotated { items, .. } | Draw::Clip { items, .. } | Draw::Turned { items, .. } => ink_width(items),
        _ => None,
    })
}

fn mask_bounds(draws: &[Draw]) -> Option<[f32; 4]> {
    draws.iter().find_map(|draw| match draw {
        Draw::MaskInk { segs, .. } => {
            let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
            for seg in segs {
                if let PathSeg::Move(x, y) | PathSeg::Line(x, y) = seg {
                    bounds[0] = bounds[0].min(*x);
                    bounds[1] = bounds[1].min(*y);
                    bounds[2] = bounds[2].max(*x);
                    bounds[3] = bounds[3].max(*y);
                }
            }
            bounds.iter().all(|v| v.is_finite()).then_some(bounds)
        }
        Draw::Figure { draws, .. } => mask_bounds(draws),
        Draw::Rotated { items, .. } | Draw::Clip { items, .. } | Draw::Turned { items, .. } => mask_bounds(items),
        _ => None,
    })
}

pub(super) fn tap_and_line(draws: &[Draw], out: &mut Vec<(f32, f32)>) {
    use wordcraft_doc::para::ShapeKind;
    use wordcraft_layout::display::Paint;
    for draw in draws {
        match draw {
            Draw::NativeInk { width, color: Rgb::BLACK, .. } => out.push((*width, *width)),
            Draw::Shape { rect, kind: ShapeKind::Ellipse, fill: Some(Paint::Solid(Rgb::BLACK, _)), .. } => out.push((rect.w, rect.h)),
            Draw::Figure { draws, .. } => tap_and_line(draws, out),
            Draw::Rotated { items, .. } | Draw::Clip { items, .. } | Draw::Turned { items, .. } => tap_and_line(items, out),
            _ => {}
        }
    }
}

#[test]
fn resized_taps_match_their_pen_strokes_in_all_placements_and_after_saves() {
    use std::sync::Arc;
    use wordcraft_doc::{Block, para::InlineObject};
    // Deliberately differ from the fixture's frame aspect ratio, so the first fit
    // is already nonuniform before the second nonuniform resize.
    let ink = r#"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="width" value="4" units="pt"/></brush><trace>0 0,50 0</trace><trace>25 12.5</trace></ink>"#;
    for placement in ["inline", "floating", "group", "canvas", "direct"] {
        let mut doc = crate::io::open_bytes("tap.docx", &document_with_ink(placement, ink)).unwrap();
        for block in &mut doc.body {
            let Block::Para(para) = Arc::make_mut(block) else { continue };
            for object in &mut para.objects {
                // Direct run-level ink has no persisted outer resize frame. Keep it as
                // an intrinsic-size control; exercise resizing on every framed placement.
                if placement != "direct"
                    && matches!(object, InlineObject::Graphic { .. } | InlineObject::Group { .. })
                    && let Some((w, h, _)) = object.frame()
                {
                    object.set_size(w * 1.5, h * 0.75);
                }
            }
        }
        let mut expected: Option<Vec<(f32, f32)>> = None;
        for _ in 0..3 {
            // Recovery must retain any fitting metadata, not just the object frame.
            doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
            let mut session = Session::new(doc);
            let layout = session.layout();
            let mut sizes = Vec::new();
            tap_and_line(&page_display(&session.doc, &layout.pages[0], &Default::default()), &mut sizes);
            let [(line, _), (tap_w, tap_h)] = sizes.as_slice() else { panic!("{placement}: {sizes:?}") };
            assert!((line - tap_w).abs() < 1e-4 && (line - tap_h).abs() < 1e-4, "{placement}: {sizes:?}");
            if let Some(before_save) = &expected {
                for ((w, h), (before_w, before_h)) in sizes.iter().zip(before_save) {
                    assert!((w - before_w).abs() < 1e-4 && (h - before_h).abs() < 1e-4, "{placement}: {before_save:?} -> {sizes:?}");
                }
            } else {
                expected = Some(sizes.clone());
            }
            let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &Default::default()).unwrap();
            assert!(!pdf.is_empty());
            let saved = crate::io::save_bytes("tap.docx", &session.doc).unwrap();
            doc = crate::io::open_bytes("tap.docx", &saved).unwrap();
            assert!(doc.passthrough.values().any(|bytes| bytes.as_slice() == ink.as_bytes()));
        }
    }
}

#[test]
fn native_pen_widths_reach_screen_and_pdf_without_legacy_clamping() {
    for width in [0.01f32, 400.0] {
        let ink = format!(
            r#"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="width" value="{width}" units="pt"/></brush><trace>0 0,50 50</trace></ink>"#
        );
        let doc = crate::io::open_bytes("width.docx", &document_with_ink("direct", &ink)).unwrap();
        let mut session = Session::new(doc);
        let layout = session.layout();
        let display = page_display(&session.doc, &layout.pages[0], &Default::default());
        let actual = ink_width(&display).unwrap();
        assert!((actual - width).abs() < 1e-4, "{width} -> {actual}");
        let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &wordcraft_pdf::PdfOptions { compress: false, ..Default::default() }).unwrap();
        let raw = String::from_utf8_lossy(&pdf);
        let tokens: Vec<_> = raw.split_ascii_whitespace().collect();
        assert!(tokens.windows(2).any(|pair| pair[1] == "w" && pair[0].parse::<f32>().is_ok_and(|w| (w - actual).abs() < 1e-4)), "width={actual}");
        assert!(!raw.contains("/Subtype /Image"));
    }
}

#[test]
fn black_and_binary_mask_pens_remain_visible_or_neutral_on_dark_pages() {
    for placement in ["inline", "floating", "group", "canvas", "direct"] {
        for (color, expected) in [("#000000", Rgb(255, 255, 255)), ("#FFFFFF", Rgb(51, 51, 51)), ("#FFFF00", Rgb(51, 51, 255))] {
            let ink = format!(
                r#"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="color" value="{color}"/><brushProperty name="width" value="12" units="pt"/><brushProperty name="tip" value="rectangle"/><brushProperty name="rasterOp" value="maskPen"/></brush><trace>0 0,50 0</trace></ink>"#
            );
            let doc = crate::io::open_bytes("mask.docx", &document_with_ink(placement, &ink)).unwrap();
            let mut session = Session::new(doc);
            let layout = session.layout();
            let page = &layout.pages[0];
            let [l, t, r, b] = mask_bounds(&page_display(&session.doc, page, &Default::default())).unwrap();
            let (x, y) = (((l + r) / 2.0) as u32, ((t + b) / 2.0) as u32);
            let opts = wordcraft_render::RenderOptions { dark: true, ..Default::default() };
            let image = wordcraft_render::render_page(&session.doc, page, 1.0, &opts);
            let pixel = image.pixel(x, y);
            let hidden = wordcraft_render::RenderOptions { display: DisplayOptions { hide_ink: true, ..Default::default() }, ..opts };
            let background = wordcraft_render::render_page(&session.doc, page, 1.0, &hidden).pixel(x, y);
            assert!(
                pixel.into_iter().zip([expected.0, expected.1, expected.2, 255]).all(|(a, b)| a.abs_diff(b) <= 2),
                "{placement} {color}: {pixel:?}"
            );
            assert_eq!(background, [51, 51, 51, 255]);
            let pdf =
                wordcraft_pdf::export_layout(&session.doc, &layout, &wordcraft_pdf::PdfOptions { compress: false, ..Default::default() }).unwrap();
            let raw = String::from_utf8_lossy(&pdf);
            assert!(raw.contains("/Multiply") && !raw.contains("/Screen") && !raw.contains("/Subtype /Image"));
        }
    }
}

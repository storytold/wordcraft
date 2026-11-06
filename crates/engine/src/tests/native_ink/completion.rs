use super::{ADVANCED_INK, DisplayOptions, Draw, Rgb, Session, document_with_ink, outline_bounds, page_display};

fn masks(draws: &[Draw]) -> Vec<&Draw> {
    draws
        .iter()
        .flat_map(|draw| match draw {
            d @ Draw::MaskInk { .. } => vec![d],
            Draw::Figure { draws, .. } => masks(draws),
            Draw::Rotated { items, .. } | Draw::Clip { items, .. } => masks(items),
            _ => Vec::new(),
        })
        .collect()
}

fn rotation(draws: &[Draw]) -> Option<(f32, f32)> {
    draws.iter().find_map(|draw| match draw {
        Draw::Rotated { cx, cy, items, .. } if outline_bounds(items).is_some() => Some((*cx, *cy)),
        Draw::Figure { draws, .. } => rotation(draws),
        Draw::Clip { items, .. } => rotation(items),
        _ => None,
    })
}

#[test]
fn mask_pen_blends_over_existing_ink_in_every_placement_and_pdf() {
    for placement in ["inline", "floating", "group", "canvas", "direct"] {
        for alpha in [0.0, 0.5] {
            let ink = format!(
                r##"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="color" value="#0000FF"/><brushProperty name="tip" value="rectangle"/><brushProperty name="width" value="12"/><brushProperty name="height" value="12"/></brush><trace>0 0,50 0</trace><brush><brushProperty name="color" value="#FFFF00"/><brushProperty name="tip" value="rectangle"/><brushProperty name="width" value="12"/><brushProperty name="height" value="12"/><brushProperty name="rasterOp" value="maskPen"/><brushProperty name="transparency" value="{alpha}" units="%"/></brush><trace>0 0,50 0,0 0</trace></ink>"##
            );
            let doc = crate::io::open_bytes("mask.docx", &document_with_ink(placement, &ink)).unwrap();
            let mut session = Session::new(doc);
            let layout = session.layout();
            let page = &layout.pages[0];
            let display = page_display(&session.doc, page, &Default::default());
            let mask = masks(&display);
            assert_eq!(mask.len(), 1);
            let Draw::MaskInk { segs, color: Rgb(255, 255, 0), .. } = mask[0] else { panic!() };
            let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
            for seg in segs {
                if let wordcraft_doc::graphic::PathSeg::Move(x, y) | wordcraft_doc::graphic::PathSeg::Line(x, y) = seg {
                    bounds[0] = bounds[0].min(*x);
                    bounds[1] = bounds[1].min(*y);
                    bounds[2] = bounds[2].max(*x);
                    bounds[3] = bounds[3].max(*y);
                }
            }
            let (x, y) = (((bounds[0] + bounds[2]) * 0.5) as u32, ((bounds[1] + bounds[3]) * 0.5) as u32);
            let image = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
            let pixel = image.pixel(x, y);
            let blue = if alpha == 0.0 { 0 } else { 128 };
            assert!(pixel[0] < 3 && pixel[1] < 3 && pixel[2].abs_diff(blue) <= 2, "{placement} {alpha}: {pixel:?}");
            let hidden = DisplayOptions { hide_ink: true, ..Default::default() };
            assert!(masks(&page_display(&session.doc, page, &hidden)).is_empty());
            let image =
                wordcraft_render::render_page(&session.doc, page, 1.0, &wordcraft_render::RenderOptions { display: hidden, ..Default::default() });
            assert_eq!(image.pixel(x, y), [255, 255, 255, 255]);
            let pdf =
                wordcraft_pdf::export_layout(&session.doc, &layout, &wordcraft_pdf::PdfOptions { compress: false, ..Default::default() }).unwrap();
            let raw = String::from_utf8_lossy(&pdf);
            assert!(raw.contains("/Multiply"));
            assert!(!raw.contains("/Subtype /Image"));
        }
    }
}

#[test]
fn displaced_inner_ink_rotates_about_its_own_center_after_resize_and_roundtrip() {
    let ink = ADVANCED_INK.replace("#2563EB", "#FF0000");
    for placement in ["inline-inner", "floating-inner"] {
        let mut doc = crate::io::open_bytes("inner.docx", &document_with_ink(placement, &ink)).unwrap();
        for resized in [false, true] {
            if resized {
                for block in &mut doc.body {
                    let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(block) else { continue };
                    for object in &mut p.objects {
                        if let wordcraft_doc::para::InlineObject::Graphic { w, h, float, .. } = object {
                            *w *= 1.5;
                            *h *= 0.75;
                            let mut spin = float.spin();
                            spin.deg = 30.0;
                            float.set_spin(spin);
                        }
                    }
                }
            }
            let mut session = Session::new(doc);
            let mut before: Option<[f32; 6]> = None;
            for _ in 0..3 {
                let layout = session.layout();
                let display = page_display(&session.doc, &layout.pages[0], &Default::default());
                let [l, t, r, b] = outline_bounds(&display).unwrap();
                let (cx, cy) = rotation(&display).unwrap();
                assert!((cx - (l + r) * 0.5).abs() < 0.002 && (cy - (t + b) * 0.5).abs() < 0.002, "{placement}, {resized}: inner-center rotation");
                if let Some(previous) = before.as_ref() {
                    assert!(
                        [l, t, r, b, cx, cy].iter().zip(previous).all(|(a, b)| (a - b).abs() < 0.002),
                        "{placement} {resized}: frame changed after save"
                    );
                } else {
                    before = Some([l, t, r, b, cx, cy]);
                }
                let image = wordcraft_render::render_page(&session.doc, &layout.pages[0], 1.0, &Default::default());
                assert!(image.pixels.as_chunks::<4>().0.iter().filter(|p| p[0] > 220 && p[1] < 220 && p[2] < 220).count() > 50);
                let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &Default::default()).unwrap();
                assert!(!String::from_utf8_lossy(&pdf).contains("/Subtype /Image"));
                let saved = crate::io::save_bytes("inner.docx", &session.doc).unwrap();
                session = Session::new(crate::io::open_bytes("inner.docx", &saved).unwrap());
            }
            doc = session.doc;
        }
    }
}

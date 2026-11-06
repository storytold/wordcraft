//! Design › Watermark on screen.

use super::*;

/// #2954: a text watermark at 80 % transparency is lighter than at 0 %, and invisible at 100 %.
#[test]
fn text_watermark_transparency() {
    let mut d = Document::new();
    let darkest = |d: &Document| {
        let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(d, &l.pages[0], 1.0, &RenderOptions::default());
        img.pixels.chunks(4).map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])).min().unwrap()
    };
    let mut wm = wordcraft_doc::Watermark { text: "WWW".into(), color: wordcraft_doc::props::Rgb::BLACK, diagonal: false, ..Default::default() };
    let mut at = |t: u8| {
        wm.set_transparency(t);
        d.settings.watermark = Some(wm.clone());
        darkest(&d)
    };
    let (opaque, half, light, gone) = (at(0), at(50), at(80), at(100));
    assert!(opaque < 30, "{opaque}");
    assert!(half > opaque + 300 && light > half + 150, "{opaque} {half} {light}");
    assert_eq!(gone, 765);
}

/// A solid `w`×`h` PNG.
fn solid_png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba(c)).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

/// The RGB of pixel (`x`, `y`) of a raster drawn at one pixel per point.
fn px(img: &Rendered, x: f32, y: f32) -> [u8; 3] {
    let i = (y as usize * img.width as usize + x as usize) * 4;
    [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

/// What Word's Washout does to a colour: the picture corrections a washed-out watermark is drawn
/// with, applied to its pixel.
fn washed(c: [u8; 3]) -> [u8; 3] {
    let mut px = [c[0], c[1], c[2], 255];
    wordcraft_doc::WatermarkPicture::default().fx().apply_rgba(&mut px);
    [px[0], px[1], px[2]]
}

fn shot(d: &Document, page: usize) -> (Rendered, wordcraft_geom::Rect) {
    let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
    let p = l.pages[page].clone();
    (render_page(d, &p, 1.0, &RenderOptions::default()), p.body)
}

/// #2954: a picture watermark is drawn centred in the text area behind the text: at Auto as
/// wide as the margins allow, at a scale as a percentage of its own size, and washed out
/// exactly as the picture corrections make it (the same path as any corrected picture).
#[test]
fn picture_watermark_in_the_margins_washed_out() {
    let blue = [20, 60, 140];
    let mut d = Document::new();
    d.media.insert("wm.png".into(), Arc::new(solid_png(40, 20, [20, 60, 140, 255])));
    let mut pic = wordcraft_doc::WatermarkPicture { media: "wm.png".into(), scale: None, washout: false };
    d.settings.watermark = Some(wordcraft_doc::Watermark { picture: Some(pic.clone()), ..Default::default() });
    let (img, body) = shot(&d, 0);
    let (cx, cy) = (body.x + body.w / 2.0, body.y + body.h / 2.0);
    assert!(near(px(&img, cx, cy), blue), "{:?}", px(&img, cx, cy));
    // Auto: the margins' width, half as tall, centred.
    assert!(near(px(&img, body.x + 3.0, cy), blue) && near(px(&img, body.right() - 3.0, cy), blue));
    assert_eq!(px(&img, body.x - 3.0, cy), [255, 255, 255]);
    assert!(near(px(&img, cx, cy - body.w / 4.0 + 3.0), blue));
    assert_eq!(px(&img, cx, cy - body.w / 4.0 - 3.0), [255, 255, 255]);
    // Washout: lighter in every channel, to the value the picture corrections give.
    pic.washout = true;
    d.settings.watermark = Some(wordcraft_doc::Watermark { picture: Some(pic.clone()), ..Default::default() });
    let got = px(&shot(&d, 0).0, cx, cy);
    assert!(near(got, washed(blue)), "{got:?} vs {:?}", washed(blue));
    assert!(got.iter().zip(blue).all(|(w, b)| *w > b));
    // 100 %: the picture's own size (40 × 20 px at 96 ppi = 30 × 15 pt).
    pic.scale = Some(100.0);
    d.settings.watermark = Some(wordcraft_doc::Watermark { picture: Some(pic), ..Default::default() });
    let (img, _) = shot(&d, 0);
    assert!(near(px(&img, cx + 13.0, cy), washed(blue)));
    assert_eq!(px(&img, cx + 17.0, cy), [255, 255, 255]);
    // A picture that can't be read draws nothing.
    d.media.insert("wm.png".into(), Arc::new(b"junk".to_vec()));
    assert_eq!(px(&shot(&d, 0).0, cx, cy), [255, 255, 255]);
}

/// #2954 review: with mirrored margins an even page's text area moves, and the picture watermark
/// with it, the same size.
#[test]
fn picture_watermark_follows_the_even_pages_mirrored_margins() {
    let blue = [20, 60, 140];
    let mut d = Document::from_text("One");
    let mut two = wordcraft_doc::Paragraph::with_text("Two", Default::default());
    two.props.page_break_before = Some(true);
    d.body.push(wordcraft_doc::para_block(two));
    d.settings.mirror_margins = true;
    d.last_section.margin_left = 126.0;
    d.last_section.margin_right = 54.0;
    d.media.insert("wm.png".into(), Arc::new(solid_png(40, 20, [20, 60, 140, 255])));
    let pic = wordcraft_doc::WatermarkPicture { media: "wm.png".into(), scale: None, washout: false };
    d.settings.watermark = Some(wordcraft_doc::Watermark { picture: Some(pic), ..Default::default() });
    let (odd, odd_body) = shot(&d, 0);
    let (even, even_body) = shot(&d, 1);
    assert!((odd_body.x - even_body.x).abs() > 50.0, "the margins mirror: {odd_body:?} {even_body:?}");
    for (img, body) in [(&odd, odd_body), (&even, even_body)] {
        let cy = body.y + body.h / 2.0;
        assert!(near(px(img, body.x + 3.0, cy), blue) && near(px(img, body.right() - 3.0, cy), blue), "{body:?}");
        assert_eq!(px(img, body.x - 3.0, cy), [255, 255, 255], "{body:?}");
        assert_eq!(px(img, body.right() + 3.0, cy), [255, 255, 255], "{body:?}");
    }
}

/// #2954: Auto is the room of the first section naming the page's header, on screen as in a PDF
/// and a saved file: a landscape section linked to a portrait section's header shows the
/// portrait-sized picture, not one re-fitted to its wider page.
#[test]
fn auto_watermark_in_a_linked_landscape_section_keeps_the_headers_size() {
    let blue = [20, 60, 140];
    let mut d = Document::new();
    let head =
        d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Head", Default::default()))]);
    let mut s1 = wordcraft_doc::SectionProps::default();
    s1.headers.default = Some(head);
    let mut one = wordcraft_doc::Paragraph::with_text("Portrait", Default::default());
    one.section = Some(Box::new(s1));
    d.body = vec![wordcraft_doc::para_block(one), wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Landscape", Default::default()))];
    d.last_section.set_landscape(true);
    d.media.insert("wm.png".into(), Arc::new(solid_png(400, 200, [20, 60, 140, 255])));
    let pic = wordcraft_doc::WatermarkPicture { media: "wm.png".into(), scale: None, washout: false };
    d.settings.watermark = Some(wordcraft_doc::Watermark { picture: Some(pic), ..Default::default() });
    let (img, body) = shot(&d, 1);
    assert!(body.w > 600.0, "a landscape page: {body:?}");
    let (cx, cy) = (body.x + body.w / 2.0, body.y + body.h / 2.0);
    // 468 pt wide, centred: inside at 230 pt from the centre, outside at 238.
    assert!(near(px(&img, cx + 230.0, cy), blue) && near(px(&img, cx - 230.0, cy), blue));
    assert_eq!(px(&img, cx + 238.0, cy), [255, 255, 255]);
    assert_eq!(px(&img, cx - 238.0, cy), [255, 255, 255]);
}

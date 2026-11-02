use super::*;
use wordcraft_layout::display::page_display;
use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

mod color_glyphs;

/// The first and last pixel rows with ink in a black-on-white [`render_text_line`] raster.
fn ink_rows(img: &Rendered) -> Option<(usize, usize)> {
    let w = img.width as usize * 4;
    let rows: Vec<usize> = img.pixels.chunks(w).enumerate().filter(|(_, r)| r.chunks(4).any(|p| p[0] < 128)).map(|(i, _)| i).collect();
    Some((*rows.first()?, *rows.last()?))
}

/// #2570: a font with a very tall ascent (Amiri Quran's is 1.8 em, for stacked marks) put its
/// font list entry's baseline below the row, so the name was cut in half, and every tall-ascent
/// font sat lower than its neighbours. The entry's capitals are now centred in the row,
/// whatever the ascent.
#[test]
fn a_tall_ascent_font_list_entry_sits_where_the_others_do() {
    // Source Sans 3 renamed (same length) with its ascent set to Amiri Quran's 1815/1000 em.
    let mut data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16("Talled Sans 3")), (b"Source Sans 3".to_vec(), b"Talled Sans 3".to_vec())] {
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    let table = |data: &[u8], tag: &[u8; 4]| {
        let n = u16::from_be_bytes([data[4], data[5]]) as usize;
        (0..n)
            .map(|k| 12 + k * 16)
            .find(|&r| &data[r..r + 4] == tag)
            .map(|r| u32::from_be_bytes(data[r + 8..r + 12].try_into().unwrap()) as usize)
            .unwrap()
    };
    let upem = u16::from_be_bytes(data[table(&data, b"head") + 18..][..2].try_into().unwrap()) as f64;
    let tall = ((upem * 1.815) as i16).to_be_bytes();
    let (hhea, os2) = (table(&data, b"hhea"), table(&data, b"OS/2"));
    for at in [hhea + 4, os2 + 68, os2 + 74] {
        data[at..at + 2].copy_from_slice(&tall);
    }
    assert!(wordcraft_fonts::FontDb::global().add_font(data) > 0);
    let db = wordcraft_fonts::FontDb::global();
    let tall_face = db.face("Talled Sans 3", "Regular");
    let normal = db.face(wordcraft_fonts::FALLBACK_FAMILY, "Regular");
    assert_eq!(tall_face.family, "Talled Sans 3");
    assert!(tall_face.ascent > normal.ascent * 1.5, "patched ascent {} vs {}", tall_face.ascent, normal.ascent);
    // The font list's entry: 13 pt text in a 244×22 pt row, 2 pt in.
    let entry = |f: &wordcraft_fonts::FontFace| ink_rows(&render_text_line(f, "Amiri Quran", 13.0, 244.0, 22.0, 2.0, 2.0));
    let (top, bottom) = entry(&normal).unwrap();
    let (tall_top, tall_bottom) = entry(&tall_face).unwrap();
    assert_eq!((tall_top, tall_bottom), (top, bottom), "same glyphs, same place in the row");
    assert!(top > 0 && bottom < 43, "the name isn't clipped: ink rows {top}..={bottom} of 44");
    // Centred: as much room above the capitals as below the descender, within a few pixels.
    let (above, below) = (top as i32, 43 - bottom as i32);
    assert!((above - below).abs() <= 6, "rows above {above}, below {below}");
}

/// #299: the caret's background is the page colour, a highlight, or the dark page.
#[test]
fn background_at_caret_follows_page_colour_highlight_and_dark_mode() {
    use wordcraft_doc::Pos;
    use wordcraft_doc::props::Highlight;
    let mut d = Document::from_text("Hello WordCraft");
    let mid = |d: &Document, off: usize| {
        let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
        let c = l.caret_on(&Pos::body(0, off), 0).unwrap();
        let page = l.pages[c.page].clone();
        (page, c.x, c.top + c.height / 2.0)
    };
    let light = RenderOptions::default();
    let dark = RenderOptions { dark: true, ..Default::default() };
    let (page, x, y) = mid(&d, 2);
    assert_eq!(background_at(&d, &page, &light, x, y), Rgb::WHITE);
    assert_eq!(background_at(&d, &page, &dark, x, y), Rgb(DARK_PAPER, DARK_PAPER, DARK_PAPER));
    assert_eq!(background_at(&d, &page, &light, f32::NAN, f32::INFINITY), Rgb::WHITE, "junk points fall back to the paper");
    d.settings.page_color = Some(Rgb(0x10, 0x10, 0x40));
    assert_eq!(background_at(&d, &page, &light, x, y), Rgb(0x10, 0x10, 0x40));
    d.settings.page_color = None;
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| c.highlight = Some(Highlight::Black)).unwrap();
    let (page, x, y) = mid(&d, 2);
    assert_eq!(background_at(&d, &page, &light, x, y), Rgb::BLACK, "inside the highlight");
    let (page, x, y) = mid(&d, 9);
    assert_eq!(background_at(&d, &page, &light, x, y), Rgb::WHITE, "past the highlight");
}

/// #299: a filled shape colours the caret's background only inside its outline: an
/// ellipse's corner shows what lies beneath, its centre the fill.
#[test]
fn background_follows_a_shapes_outline_not_its_box() {
    let opts = RenderOptions::default();
    let ellipse = Draw::Shape {
        rect: wordcraft_geom::Rect::new(100.0, 100.0, 200.0, 100.0),
        kind: ShapeKind::Ellipse,
        fill: Some(wordcraft_layout::display::Paint::solid(Rgb::BLACK)),
        stroke: None,
        stroke_width: 0.0,
        effects: Default::default(),
        corner: None,
        dash: Vec::new(),
        cap: Default::default(),
    };
    let at = |x: f32, y: f32| {
        let mut bg = [255.0; 3];
        blend_backgrounds(std::slice::from_ref(&ellipse), wordcraft_geom::Point::new(x, y), &opts, &mut bg, 0);
        bg
    };
    assert_eq!(at(200.0, 150.0), [0.0; 3], "the centre is the fill");
    assert_eq!(at(103.0, 103.0), [255.0; 3], "the box's corner is outside the ellipse");
    assert_eq!(at(350.0, 150.0), [255.0; 3], "outside the box");
}

#[test]
fn renders_text_dark_pixels() {
    let d = Document::from_text("Hello WordCraft");
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
    assert_eq!((img.width, img.height), (612, 792));
    // Somewhere in the first text line there are dark pixels; the margins are white.
    let dark = (72..300).flat_map(|x| (72..100).map(move |y| (x, y))).filter(|(x, y)| img.pixel(*x, *y)[0] < 128).count();
    assert!(dark > 30, "dark {dark}");
    assert_eq!(img.pixel(5, 5), [255, 255, 255, 255]);
    assert!(!img.to_png().is_empty());
}

/// Table Layout › Text Direction (#226): a turned cell's text is painted running down the
/// cell, not across it.
#[test]
fn turned_cell_text_is_painted_turned() {
    use wordcraft_doc::props::TextDirection;
    let ink = |dir: TextDirection| {
        let mut d = Document::from_text("before\nafter");
        let mut t = wordcraft_doc::Table::new(1, 2, 400.0);
        t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("WWWWWWWWWWWW", Default::default()))];
        t.rows[0].cells[0].props.text_direction = dir;
        d.insert_block(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(1), wordcraft_doc::Block::Table(t)).unwrap();
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
        // Dark pixels inside the first cell (x 72..272, clear of its borders), well below
        // the first text line.
        (80..264).flat_map(|x| (140..190).map(move |y| (x, y))).filter(|(x, y)| img.pixel(*x, *y)[0] < 128).count()
    };
    assert_eq!(ink(TextDirection::Horizontal), 0, "horizontal text stays in its line");
    assert!(ink(TextDirection::Down) > 30, "down");
    assert!(ink(TextDirection::Up) > 30, "up");
}

/// Shape Effects (#275): an outer shadow paints dark pixels beside the shape on the side it is
/// cast to, and none on the other; a glow surrounds it.
#[test]
fn shape_shadow_is_cast_offset_from_the_shape() {
    use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
    use wordcraft_doc::para::InlineObject;
    let draw = |effects: ShapeEffects| {
        let mut d = Document::from_text("");
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 100.0,
            h: 60.0,
            fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(0, 0, 255))),
            stroke: None,
            stroke_width: 0.0,
            float: Default::default(),
            story: None,
            freeform: None,
            effects,
            extra: Default::default(),
        };
        let at = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
        d.insert_object(&at, shape, &Default::default()).unwrap();
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        render_page(&d, &l.pages[0], 1.0, &RenderOptions::default())
    };
    let plain = draw(ShapeEffects::default());
    let blue = |img: &Rendered| {
        let px: Vec<(u32, u32)> = (0..612)
            .flat_map(|x| (0..400).map(move |y| (x, y)))
            .filter(|(x, y)| matches!(img.pixel(*x, *y), [r, _, b, _] if r < 40 && b > 200))
            .collect();
        let (x0, x1) = (px.iter().map(|p| p.0).min().unwrap(), px.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (px.iter().map(|p| p.1).min().unwrap(), px.iter().map(|p| p.1).max().unwrap());
        (x0, y0, x1, y1)
    };
    let (x0, y0, x1, y1) = blue(&plain);
    assert!(x1 - x0 > 90 && y1 - y0 > 50, "{:?}", (x0, y0, x1, y1));
    // Grey (not white, not blue) pixels in a strip just outside an edge.
    let grey = |img: &Rendered, xs: std::ops::Range<u32>, ys: std::ops::Range<u32>| {
        xs.flat_map(|x| ys.clone().map(move |y| (x, y)))
            .filter(|(x, y)| matches!(img.pixel(*x, *y), [r, g, b, _] if r < 200 && r == g && g == b))
            .count()
    };
    let right = |img: &Rendered| grey(img, x1 + 2..x1 + 6, y0 + 10..y1);
    let left = |img: &Rendered| grey(img, x0.saturating_sub(6)..x0.saturating_sub(2), y0 + 10..y1);
    assert_eq!((right(&plain), left(&plain)), (0, 0));
    let shadow = Shadow { color: Rgb::BLACK, transparency: 30.0, blur: 2.0, distance: 8.0, angle: 0.0, rot_with_shape: false };
    let img = draw(ShapeEffects { shadow: Some(shadow), ..Default::default() });
    assert_eq!(blue(&img), (x0, y0, x1, y1), "the shape itself is unchanged");
    assert!(right(&img) > 100, "shadow to the right: {}", right(&img));
    assert_eq!(left(&img), 0, "nothing on the left");
    let img = draw(ShapeEffects { glow: Some(Glow { color: Rgb(255, 0, 0), size: 6.0, transparency: 0.0 }), ..Default::default() });
    let reddish = |x: u32, y: u32| matches!(img.pixel(x, y), [r, g, _, _] if r > 200 && g < 200);
    assert!(reddish(x1 + 2, (y0 + y1) / 2) && reddish(x0 - 2, (y0 + y1) / 2), "glow on both sides");
}

/// #1500: gradient fills shade across the shape, circular ones from the centre out, and a
/// transparent fill lets what is behind show through.
#[test]
fn shape_gradients_and_transparency_are_drawn() {
    use std::sync::Arc;
    use wordcraft_doc::fill::{Fill, Gradient, GradientFlip, GradientKind, PathShape};
    use wordcraft_doc::para::{Float, InlineObject, Wrap};
    use wordcraft_doc::wordart::GradStop;
    let shape = |fill: Fill, x: f32| InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 100.0,
        h: 100.0,
        fill: Some(fill),
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap: Wrap::InFrontOfText, x, y: 0.0, ..Default::default() },
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let stops = vec![GradStop { pos: 0.0, color: Rgb::BLACK, transparency: 0.0 }, GradStop { pos: 100.0, color: Rgb::WHITE, transparency: 0.0 }];
    let grad = |kind| Fill::Gradient(Arc::new(Gradient { stops: stops.clone(), kind, tile: None, rot_with_shape: true, flip: GradientFlip::None }));
    let mut d = Document::from_text("");
    let at = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
    // Left to right: top-to-bottom linear, circular, navy under 30%-opaque white.
    let objs = [
        shape(grad(GradientKind::Linear { angle: 90.0, scaled: false }), 0.0),
        shape(grad(GradientKind::Path { shape: PathShape::Circle, focus: [50.0; 4] }), 150.0),
        shape(Fill::solid(Rgb(0, 0, 0x80)), 300.0),
        shape(Fill::Solid { color: Rgb::WHITE, transparency: 70.0 }, 300.0),
    ];
    for o in objs.into_iter().rev() {
        d.insert_object(&at, o, &Default::default()).unwrap();
    }
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
    // Each shape's frame (the dark corner of the linear one is its top-left pixel).
    let dark = |x0: u32, x1: u32| -> Vec<(u32, u32)> {
        (x0..x1).flat_map(|x| (0..400).map(move |y| (x, y))).filter(|(x, y)| img.pixel(*x, *y)[0] < 40).collect()
    };
    let lin = dark(60, 200);
    let (lx, ly) = (lin.iter().map(|p| p.0).min().unwrap(), lin.iter().map(|p| p.1).min().unwrap());
    let luma = |x: u32, y: u32| img.pixel(x, y)[0];
    assert!(
        luma(lx + 50, ly + 5) < luma(lx + 50, ly + 50) && luma(lx + 50, ly + 50) < luma(lx + 50, ly + 95),
        "dark at the top, light at the bottom"
    );
    let (cx, cy) = (lx + 150 + 50, ly + 50);
    assert!(luma(cx, cy) < 30 && luma(cx - 48, cy - 48) > 200, "black at the centre, white at the corners");
    let [r, _, b, _] = img.pixel(lx + 300 + 50, cy);
    assert!((150..230).contains(&b) && (60..120).contains(&r), "white over navy blends: {r} {b}");
}

#[test]
fn stem_darkening_follows_the_macos_curve() {
    assert_eq!(stem_darkening(0.0), 0.0);
    assert_eq!(stem_darkening(f64::NAN), 0.0);
    assert_eq!(stem_darkening(-5.0), 0.0);
    // Small text grows in proportion to its pixel size...
    assert!((stem_darkening(10.0) - 10.0 * 0.0121 * 2.25 / 2.0).abs() < 1e-9);
    // ...body text on a Retina screen hits the 0.3 px cap...
    assert_eq!(stem_darkening(30.0), 0.3);
    assert_eq!(stem_darkening(72.0), 0.3);
    // ...and display sizes aren't darkened at all.
    assert_eq!(stem_darkening(72.5), 0.0);
}

/// Ink in a raster: the sum of how far each pixel is from white.
fn ink(img: &Rendered) -> u64 {
    img.pixels.as_chunks::<4>().0.iter().map(|p| 255 - p[0] as u64).sum()
}

#[test]
fn text_darkening_adds_ink_on_screen_only() {
    let d = Document::from_text("Marketing analytics lead with twelve years of measurement work.");
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let plain = render_page(&d, &l.pages[0], 2.0, &RenderOptions::default());
    let dark = render_page(&d, &l.pages[0], 2.0, &RenderOptions { text_darkening: true, ..Default::default() });
    let (a, b) = (ink(&plain), ink(&dark));
    // About 0.3 px more on every edge of an 11 pt line at 2x: clearly more ink, not bold.
    assert!(b as f64 > a as f64 * 1.08 && (b as f64) < a as f64 * 1.6, "plain {a}, darkened {b}");
    // Off by default: exports and thumbnails are untouched.
    assert!(!RenderOptions::default().text_darkening);
}

#[test]
fn page_picture_is_painted_behind_the_text() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 200, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut d = Document::new();
    d.media.insert("bg.png".into(), Arc::new(png));
    for kind in ["tile", "frame"] {
        d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: "bg.png".into(), fill_type: kind.into(), title: String::new() });
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
        let i = ((300 * img.width + 300) * 4) as usize;
        assert_eq!(&img.pixels[i..i + 3], &[10, 200, 30], "{kind}");
    }
    // An undecodable picture leaves the paper alone.
    d.media.insert("bg.png".into(), Arc::new(b"junk".to_vec()));
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
    assert_eq!(&img.pixels[0..3], &[255, 255, 255]);
}

/// Read Mode's Page Color: the reading paper replaces the document's page color and page
/// picture on screen, in the raster and under the caret; the text keeps its color on Sepia.
#[test]
fn read_paper_replaces_the_page_background() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 200, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut d = Document::from_text("Hello WordCraft");
    d.media.insert("bg.png".into(), Arc::new(png));
    d.settings.page_color = Some(Rgb(0x10, 0x10, 0x40));
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let page = &l.pages[0];
    let at = |img: &Rendered, x: u32, y: u32| {
        let i = ((y * img.width + x) * 4) as usize;
        [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2]]
    };
    let sepia = RenderOptions { read_paper: Some(SEPIA_PAPER), ..Default::default() };
    let white = RenderOptions { read_paper: Some(Rgb::WHITE), ..Default::default() };
    assert_eq!(at(&render_page(&d, page, 1.0, &RenderOptions::default()), 5, 5), [0x10, 0x10, 0x40], "the page color");
    assert_eq!(at(&render_page(&d, page, 1.0, &sepia), 5, 5), [0xE2, 0xD9, 0xD0]);
    assert_eq!(at(&render_page(&d, page, 1.0, &white), 5, 5), [255, 255, 255]);
    assert_eq!(background_at(&d, page, &sepia, 5.0, 5.0), SEPIA_PAPER);
    assert_eq!(background_at(&d, page, &white, 5.0, 5.0), Rgb::WHITE);
    let inverse = RenderOptions { read_paper: Some(Rgb::WHITE), dark: true, ..Default::default() };
    assert_eq!(background_at(&d, page, &inverse, 5.0, 5.0), Rgb(DARK_PAPER, DARK_PAPER, DARK_PAPER));
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: "bg.png".into(), fill_type: "tile".into(), title: String::new() });
    assert_eq!(at(&render_page(&d, page, 1.0, &RenderOptions::default()), 300, 300), [10, 200, 30], "the page picture");
    assert_eq!(at(&render_page(&d, page, 1.0, &sepia), 300, 300), [0xE2, 0xD9, 0xD0]);
    // The text is drawn in its own color on sepia paper: its darkest pixel is within 8 levels of its value on white.
    d.settings.page_picture = None;
    d.settings.page_color = None;
    let darkest = |img: &Rendered| img.pixels.chunks(4).map(|p| p[0].min(p[1]).min(p[2])).min().unwrap_or(255);
    let plain = render_page(&d, page, 2.0, &RenderOptions::default());
    let on_sepia = render_page(&d, page, 2.0, &sepia);
    assert!(darkest(&plain) < 0x40 && darkest(&on_sepia) <= darkest(&plain) + 8, "{} {}", darkest(&plain), darkest(&on_sepia));
}

/// A PNG signature, an IHDR declaring `w`×`h` 8-bit RGBA and a short IDAT (all CRCs valid, so
/// the header parses), with no real pixel data.
fn ihdr_only_png(w: u32, h: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        let mut crc = 0xFFFF_FFFFu32;
        for b in &body {
            crc ^= *b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&(!crc).to_be_bytes());
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &[0x78, 0x9c, 0, 0]);
    out
}

#[test]
fn decode_budget_is_checked_from_the_header() {
    let mut buf = Vec::new();
    let img = image::RgbaImage::from_fn(16, 8, |x, y| image::Rgba([x as u8, y as u8, 9, 255]));
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    // Decode, the RGBA conversion and the premultiplied copy are charged: two buffers at peak.
    assert!(decode_pixmap_within(&buf, 1024).is_some());
    assert!(decode_pixmap_within(&buf, 1023).is_none());
    // 16-bit RGBA decodes to 8 bytes a pixel, alive with its RGBA8 copy: 12 bytes a pixel.
    let mut wide = Vec::new();
    let img16 = image::ImageBuffer::<image::Rgba<u16>, _>::from_fn(16, 8, |x, y| image::Rgba([x as u16 * 999, y as u16, 9, 65535]));
    image::DynamicImage::ImageRgba16(img16).write_to(&mut std::io::Cursor::new(&mut wide), image::ImageFormat::Png).unwrap();
    assert!(decode_pixmap_within(&wide, 1024).is_none(), "two RGBA8 buffers' worth is not enough for RGBA16");
    assert!(decode_pixmap_within(&wide, 1536).is_some());
    // A header (valid CRC) claiming 60000x60000 with no pixel data reads fine, then the budget refuses it.
    let hdr = ihdr_only_png(60_000, 60_000);
    assert_eq!(image_size(&hdr), Some((60_000, 60_000)));
    assert!(decode_pixmap(&hdr).is_none());
}

/// A `Rendered` whose pixels don't match its claimed size isn't padded out to that size: a
/// 60000×60000 claim over no pixels would allocate 14 GB. It, and any mismatch, encodes to
/// nothing (JPEG used to assert on the length).
#[test]
fn encoders_refuse_pixels_that_dont_match_the_size() {
    let huge = Rendered { width: 60_000, height: 60_000, pixels: Vec::new() };
    assert!(huge.to_png().is_empty());
    assert!(huge.to_jpeg(80).is_empty());
    let short = Rendered { width: 2, height: 2, pixels: vec![255; 4] };
    assert!(short.to_png().is_empty());
    assert!(short.to_jpeg(80).is_empty());
    let long = Rendered { width: 1, height: 1, pixels: vec![255; 8] };
    assert!(long.to_png().is_empty());
    let ok = Rendered { width: 2, height: 2, pixels: vec![255; 16] };
    assert!(!ok.to_png().is_empty());
    assert!(!ok.to_jpeg(80).is_empty());
}

#[test]
fn page_raster_is_capped_by_total_pixels() {
    let d = Document::new();
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let opts = RenderOptions::default();
    let big = render_region_capped(&d, &l.pages[0], 3000, 2000, Affine::scale(4.0), &opts, 100_000);
    assert!(big.width as u64 * big.height as u64 <= 100_000, "{}x{}", big.width, big.height);
    assert!(big.width > 300 && big.height > 200);
    assert_eq!(big.pixels.len() as u64, big.width as u64 * big.height as u64 * 4);
    let ok = render_region_capped(&d, &l.pages[0], 300, 200, Affine::IDENTITY, &opts, 100_000);
    assert_eq!((ok.width, ok.height), (300, 200));
}

#[test]
fn hostile_scale_is_clamped() {
    let d = Document::new();
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let img = render_page(&d, &l.pages[0], f32::NAN, &RenderOptions::default());
    assert_eq!(img.width, 612);
    let big = render_page(&d, &l.pages[0], 1e9, &RenderOptions::default());
    assert!(big.width <= MAX_SIDE);
    assert!(decode_pixmap(b"not an image").is_none());
}

/// A JPEG whose headers say progressive (SOF2, a DC-only first scan) and `w` × `h`, its three
/// components unsubsampled, with no pixel data that size.
fn progressive_jpeg_header(w: u16, h: u16) -> Vec<u8> {
    let mut jpeg = Vec::new();
    image::RgbImage::new(16, 16).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
    let sof = jpeg.windows(2).position(|m| m == [0xFF, 0xC0]).unwrap();
    jpeg[sof + 1] = 0xC2;
    jpeg[sof + 5..sof + 7].copy_from_slice(&h.to_be_bytes());
    jpeg[sof + 7..sof + 9].copy_from_slice(&w.to_be_bytes());
    for c in 0..3 {
        jpeg[sof + 11 + 3 * c] = 0x11;
    }
    let sos = jpeg.windows(2).position(|m| m == [0xFF, 0xDA]).unwrap();
    let n = usize::from(jpeg[sos + 4]);
    jpeg[sos + 6 + 2 * n] = 0;
    jpeg
}

#[test]
fn progressive_jpeg_coefficients_count_toward_the_decode_budget() {
    // Two RGBA buffers of 7900 × 7900 fit 512 MB, but RGB output beside three full-size
    // coefficient planes (9 bytes a pixel) doesn't (#3644).
    let jpeg = progressive_jpeg_header(7900, 7900);
    assert!(wordcraft_geom::decode_fits(7900, 7900, wordcraft_geom::MAX_DECODED_BYTES));
    assert!(decode_pixmap(&jpeg).is_none());
    let mut baseline = Vec::new();
    image::RgbImage::new(64, 64).write_to(&mut std::io::Cursor::new(&mut baseline), image::ImageFormat::Jpeg).unwrap();
    assert!(decode_pixmap(&baseline).is_some(), "other JPEGs still draw");
}

/// #3665: a picture too large for the whole cache is kept at half of it, so it is cached
/// instead of decoded again on every render.
#[test]
fn pictures_too_large_for_the_cache_are_kept_smaller() {
    let px = vello_cpu::color::PremulRgba8 { r: 40, g: 80, b: 120, a: 200 };
    // 6000 × 6000 is 144 MB of RGBA, past the 128 MB cache.
    let big = Pixmap::from_parts(vec![px; 6000 * 6000], 6000, 6000);
    let small = cacheable(big);
    let bytes = small.width() as usize * small.height() as usize * 4;
    assert!(bytes <= CACHE_BYTES / 2, "{} x {}", small.width(), small.height());
    assert!(small.width() >= 3000 && small.width() == small.height(), "aspect kept");
    assert!(small.data().iter().all(|p| (p.r, p.g, p.b, p.a) == (40, 80, 120, 200)), "flat stays flat");
    let mut cache = MediaCache { map: HashMap::new(), bytes: 0 };
    // A source file larger than the whole cache (an uncompressed BMP, say) doesn't count against it.
    let src = Arc::new(vec![0u8; CACHE_BYTES + 1]);
    cache.insert("big", &src, Some(Arc::new(small)), bytes);
    assert!(cache.get("big", &src).is_some(), "cached");
    // Pictures the cache holds as they are keep their size.
    assert_eq!(cacheable(Pixmap::from_parts(vec![px; 100 * 50], 100, 50)).width(), 100);
}

#[test]
fn shrinking_averages_premultiplied_pixels() {
    let p = |v: u8| vello_cpu::color::PremulRgba8 { r: v, g: v, b: v, a: 255 };
    let pm = Pixmap::from_parts(vec![p(0), p(100), p(200), p(40)], 4, 1);
    let out = shrink(&pm, 2).unwrap();
    assert_eq!((out.width(), out.height()), (2, 1));
    assert_eq!(out.data().iter().map(|q| q.r).collect::<Vec<_>>(), [50, 120]);
    assert!(shrink(&pm, 4).is_none() && shrink(&pm, 0).is_none());
}

#[test]
fn dimmed_header_image_renders() {
    let mut d = Document::from_text("body");
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let media = d.add_media(png, "png");
    let mut hp = wordcraft_doc::Paragraph::new();
    let obj = wordcraft_doc::para::InlineObject::Image {
        media,
        w: 40.0,
        h: 40.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    hp.insert_object(0, obj, &Default::default()).unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
    d.last_section.headers.default = Some(id);
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let opts = RenderOptions::default();
    assert!(opts.display.dim_header);
    let img = render_page(&d, &l.pages[0], 1.0, &opts);
    assert!(!img.to_png().is_empty());
}

/// #2892: a rule is stroked with flat ends, as the PDF export and Word draw them. Round caps
/// used to stick half the width out past every end — a blob over the neighbouring border at
/// each corner of a table with thick borders, while the exported PDF had square corners.
#[test]
fn a_thick_rule_ends_flat_not_round() {
    let d = Document::new();
    let rule = |stroke| Draw::Line { x0: 10.0, y0: 10.0, x1: 30.0, y1: 10.0, width: 8.0, color: Rgb::BLACK, stroke, alpha: 1.0, text_bg: None };
    let shot = |stroke| render_items(&d, &[rule(stroke)], wordcraft_geom::Rect::new(0.0, 0.0, 40.0, 20.0), 1.0, &RenderOptions::default());
    let ink = |img: &Rendered, x: u32, y: u32| img.pixel(x, y)[3] > 128;
    // An 8 pt rule from x=10 to x=30 covers y 6..14, and nothing beyond either end.
    let img = shot(Stroke::Solid);
    assert!(ink(&img, 15, 10), "the rule itself is drawn");
    assert!(!ink(&img, 6, 10) && !ink(&img, 33, 10), "no round cap past x=10 or x=30");
    // The double style's two strands end flat as well: its upper strand is centred on y=2.8
    // with a 4.8 pt width, so x=31 lies in the half-disc a round cap would add.
    let img = shot(Stroke::Double);
    assert!(ink(&img, 15, 3), "the double rule's upper strand is drawn");
    assert!(!ink(&img, 31, 3), "the upper strand ends at x=30");
}

/// A dimmed header is its full-strength picture faded once, as Word dims it: where a header
/// table's borders meet they are the tone of the borders, not darker.
#[test]
fn dimmed_header_borders_dont_darken_where_they_meet() {
    let mut d = Document::from_text("");
    let t = wordcraft_doc::Table::new(2, 2, 200.0);
    let blocks = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t)), wordcraft_doc::para_block(wordcraft_doc::Paragraph::new())];
    let id = d.add_part(wordcraft_doc::PartKind::Header, blocks);
    d.last_section.headers.default = Some(id);
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let render = |dim_header: bool| {
        let mut opts = RenderOptions::default();
        opts.display.dim_header = dim_header;
        render_page(&d, &l.pages[0], 3.0, &opts)
    };
    let (full, dim) = (render(false), render(true));
    let mut ink = 0;
    for y in 0..full.height {
        for x in 0..full.width {
            let (f, g) = (full.pixel(x, y)[0] as f32, dim.pixel(x, y)[0] as f32);
            ink += usize::from(f < 128.0);
            let want = 255.0 - 0.45 * (255.0 - f);
            assert!((g - want).abs() <= 2.0, "({x}, {y}): {g} dimmed, {want} wanted ({f} at full strength)");
        }
    }
    assert!(ink > 100, "the borders are drawn: {ink}");
}

/// #2946: a dimmed header's items are faded as one group, so two of its lines that cross come
/// out the same tone as the lines themselves, and so does a border two cells share. Faded item
/// by item, the crossing kept 0.55² of the background instead of 0.55, and a shared border read
/// darker than the outer ones.
#[test]
fn dimmed_header_borders_keep_one_tone_where_they_meet() {
    let mut d = Document::from_text("Body");
    // Two rows and two columns: the middle border is drawn by both cells beside it, and a cell's
    // top and left rules cross at its top-left corner.
    let header = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(wordcraft_doc::Table::new(2, 2, 300.0)))];
    let id = d.add_part(wordcraft_doc::PartKind::Header, header);
    d.last_section.headers.default = Some(id);
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let page = &l.pages[0];
    let rules: Vec<(f32, f32, f32, f32)> = page
        .header
        .iter()
        .filter_map(|it| if let wordcraft_layout::Placed::Rule { x0, y0, x1, y1, .. } = it { Some((*x0, *y0, *x1, *y1)) } else { None })
        .collect();
    let (vx, vy0, _, vy1) = *rules.iter().filter(|(x0, _, x1, _)| x0 == x1).min_by(|a, b| a.0.total_cmp(&b.0)).expect("a vertical rule");
    let (hx0, hy, hx1, _) = *rules.iter().find(|(_, y0, _, y1)| y0 == y1 && (y0 - vy0).abs() < 0.01).expect("the horizontal rule at the cell's top");
    assert!(hx0 <= vx && vx <= hx1 && vy0 <= hy && hy <= vy1, "the rules cross: ({hx0}, {hy}, {hx1}) across ({vx}, {vy0}, {vy1})");
    let bottom = rules.iter().filter(|(_, y0, _, y1)| y0 == y1).map(|(_, y, _, _)| *y).fold(f32::NEG_INFINITY, f32::max);
    let row = hy + (bottom - hy) * 0.25; // inside the first row, clear of any rule across it
    // The darkest pixel in a small box: the tone of a line, however it is antialiased.
    let scale = 8.0;
    let darkest = |img: &Rendered, cx: f32, cy: f32| {
        let (px, py) = ((cx * scale) as i64, (cy * scale) as i64);
        (px - 2..=px + 2)
            .flat_map(|x| (py - 2..=py + 2).map(move |y| (x, y)))
            .filter(|(x, y)| *x >= 0 && *y >= 0)
            .map(|(x, y)| img.pixel(x as u32, y as u32)[0])
            .min()
            .expect("a pixel")
    };
    let dimmed = render_page(&d, page, scale, &RenderOptions::default());
    let undimmed = RenderOptions { display: DisplayOptions { dim_header: false, ..Default::default() }, ..Default::default() };
    let undimmed = render_page(&d, page, scale, &undimmed);
    // One border, well clear of any crossing or shared edge: the tone the others must keep.
    let line = darkest(&dimmed, vx + 30.0, hy);
    assert!(line > 0, "the dimmed border is drawn, faded: {line}");
    assert_eq!(darkest(&dimmed, vx, hy), line, "the crossing is darker than the line");
    assert_eq!(darkest(&dimmed, hx0 + 150.0, row), line, "a shared border is darker than the outer ones");
    assert_eq!(darkest(&undimmed, vx, hy), darkest(&undimmed, vx + 30.0, hy), "undimmed too");
    assert!(darkest(&undimmed, vx + 30.0, hy) < line, "the header is faded");
}

#[test]
fn dark_page_inverts_lightness() {
    assert_eq!(invert_lightness(Rgb::WHITE), Rgb::BLACK);
    assert_eq!(invert_lightness(Rgb::BLACK), Rgb::WHITE);
    // Pure hues keep their colour; dark blue turns light blue.
    assert_eq!(invert_lightness(Rgb(255, 0, 0)), Rgb(255, 0, 0));
    assert_eq!(invert_lightness(Rgb(0, 0, 0x80)), Rgb(0x7F, 0x7F, 0xFF));
    let d = Document::from_text("Hello WordCraft");
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let opts = RenderOptions { dark: true, ..Default::default() };
    assert_eq!(opts.ink(Rgb::WHITE), Rgb(DARK_PAPER, DARK_PAPER, DARK_PAPER));
    assert_eq!(opts.ink(Rgb::BLACK), Rgb::WHITE);
    let img = render_page(&d, &l.pages[0], 1.0, &opts);
    assert_eq!(img.pixel(5, 5), [DARK_PAPER, DARK_PAPER, DARK_PAPER, 255]);
    let light = (72..300).flat_map(|x| (72..100).map(move |y| (x, y))).filter(|(x, y)| img.pixel(*x, *y)[0] > 128).count();
    assert!(light > 30, "light {light}");
}

/// #2534: legacy font effects are drawn: outlined letters are hollow; shadowed and embossed
/// ones have a grey copy down-right, imprinted ones up-left.
#[test]
fn legacy_font_effects_are_drawn() {
    use wordcraft_doc::CharProps;
    use wordcraft_doc::wordart::{TextEffects, TextFill};
    const SIZE: f32 = 40.0;
    const SCALE: f32 = 2.0;
    const MIN_DIRECTION_PIXELS: usize = 100;
    let shot = |c: CharProps, offset: f32| {
        let mut d = Document::from_text("x");
        let cp = CharProps { size: Some(SIZE), bold: Some(true), ..c };
        d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("HHHH", cp))];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        render_area(&d, &l.pages[0], wordcraft_geom::Rect::new(-offset, -offset, 400.0, 150.0), SCALE, &RenderOptions::default())
    };
    let counts = |img: &Rendered| {
        let (mut dark, mut grey) = (0usize, 0usize);
        for (x, y) in (100..800).flat_map(|x| (100..300).map(move |y| (x, y))) {
            let v = img.pixel(x, y)[0];
            dark += usize::from(v < 60);
            grey += usize::from((120..=136).contains(&v));
        }
        (dark, grey)
    };
    let (plain_dark, plain_grey) = counts(&shot(CharProps::default(), 0.0));
    assert!(plain_dark > 2000, "plain {plain_dark}");
    let (dark, _) = counts(&shot(CharProps { outline: Some(true), ..Default::default() }, 0.0));
    assert!(dark < plain_dark / 3, "outline is hollow: {dark} vs {plain_dark}");
    // An explicit fill keeps the reference on the same Art path as legacy effects.
    let reference = || CharProps {
        text_effects: Some(Box::new(TextEffects { fill: Some(TextFill::Solid { color: Rgb::BLACK, transparency: 0.0 }), ..Default::default() })),
        ..Default::default()
    };
    let base = shot(reference(), 0.0);
    for (c, sign) in [
        (CharProps { shadow: Some(true), ..Default::default() }, 1.0),
        (CharProps { emboss: Some(true), ..Default::default() }, 1.0),
        (CharProps { engrave: Some(true), ..Default::default() }, -1.0),
    ] {
        let img = shot(c, 0.0);
        let (_, grey) = counts(&img);
        assert!(grey > plain_grey * 2, "a grey copy: {grey} vs {plain_grey}");
        // A 2pt diagonal displacement at 40pt has sqrt(2)pt on each axis.
        // Check the visible copy against its translated mask, accounting for occlusion.
        let offset = sign * std::f32::consts::SQRT_2;
        let expected = shot(reference(), offset);
        let opposite = shot(reference(), -offset);
        let (mut visible, mut directional) = (0usize, 0usize);
        for (x, y) in (100..800).flat_map(|x| (100..300).map(move |y| (x, y))) {
            let v = img.pixel(x, y)[0];
            if !(120..=136).contains(&v) || base.pixel(x, y)[0] != 255 {
                continue;
            }
            visible += 1;
            assert!(expected.pixel(x, y)[0] < 255, "grey outside translated Art mask: sign {sign}, pixel {x},{y}");
            directional += usize::from(expected.pixel(x, y)[0] < 60 && opposite.pixel(x, y)[0] == 255);
        }
        assert!(directional > MIN_DIRECTION_PIXELS, "directional grey: sign {sign}, {directional} of {visible}");
        eprintln!("spatial shadow sign={sign} grey={grey} visible={visible} directional={directional}");
    }
}

/// A document whose only text is a yellow-highlighted run.
fn yellow_highlight_doc() -> (Document, wordcraft_layout::DocLayout) {
    let cp = wordcraft_doc::CharProps { highlight: Some(wordcraft_doc::props::Highlight::Yellow), size: Some(24.0), ..Default::default() };
    let mut d = Document::from_text("x");
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("HHHH HHHH", cp))];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    (d, l)
}

/// #1252: on a dark page a highlight is drawn dimmed (keeping its hue) so the white text on it
/// reads; a light page is unchanged.
#[test]
fn dark_page_dims_a_highlight_raster() {
    let (d, l) = yellow_highlight_doc();
    let items = page_display(&d, &l.pages[0], &DisplayOptions::default());
    let r = items
        .iter()
        .find_map(|i| match i {
            Draw::Fill { rect, color, .. } if *color == Rgb(255, 255, 0) => Some(*rect),
            _ => None,
        })
        .unwrap();
    // Just inside the top-left corner of the highlight: above the capitals, so pure background.
    let (px, py) = ((r.x + 1.5) as u32, (r.y + 1.5) as u32);
    let light = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
    assert_eq!(light.pixel(px, py), [255, 255, 0, 255], "light page keeps bright yellow");
    let dark = render_page(&d, &l.pages[0], 1.0, &RenderOptions { dark: true, ..Default::default() });
    let [r8, g8, b8, _] = dark.pixel(px, py);
    assert!(r8 < 0x60 && g8 < 0x60, "dark page highlight is dim, got {:?}", (r8, g8, b8));
    assert!(r8.abs_diff(g8) <= 4 && g8 > b8.saturating_add(0x20), "dark page highlight stays yellow in hue, got {:?}", (r8, g8, b8));
}

/// #1252: on a dark page every highlight colour is dimmed so the text drawn on it reads
/// (Automatic and explicit black text both draw white); a light page is the identity.
#[test]
fn dark_page_text_backgrounds_keep_text_readable() {
    use wordcraft_doc::props::{Highlight, TextColor};
    use wordcraft_layout::display::text_color;
    let dark = RenderOptions { dark: true, ..Default::default() };
    let light = RenderOptions::default();
    let colours: Vec<Rgb> = Highlight::ALL.iter().filter_map(|h| h.rgb()).collect();
    assert_eq!(colours.len(), 16);
    for c in colours {
        let bg = dark.ink_bg(c);
        for (what, tc) in [("automatic", text_color(&TextColor::Auto, Some(c))), ("black", Rgb::BLACK)] {
            let fg = dark.ink_text(tc, Some(c));
            assert_eq!(fg, Rgb::WHITE, "{what} text on {c:?} draws white");
            let k = contrast(fg, bg);
            assert!(k >= 4.5, "{what} text on {c:?}: {fg:?} on {bg:?} has contrast {k:.2}");
        }
        assert_eq!(light.ink_bg(c), c, "light page keeps {c:?}");
        assert_eq!(light.ink_text(c, Some(c)), c, "light page keeps text {c:?}");
    }
    // Yellow turns a dim olive (~#4a4a10), still yellow in hue.
    let Rgb(r, g, b) = dark.ink_bg(Rgb(255, 255, 0));
    assert!(r.abs_diff(0x4A) <= 8 && g.abs_diff(0x4A) <= 8 && b.abs_diff(0x10) <= 8, "yellow → {:?}", (r, g, b));
    assert!(r == g && g > b);
    // Text off a highlight keeps the plain inversion.
    assert_eq!(dark.ink_text(Rgb::WHITE, None), dark.ink(Rgb::WHITE));
}

/// #1252 review: explicit coloured text (pure blue, red, every highlight colour) on every
/// dimmed highlight still reaches 4.5:1 on a dark page.
#[test]
fn dark_page_coloured_text_on_highlights_reads() {
    use wordcraft_doc::props::Highlight;
    let dark = RenderOptions { dark: true, ..Default::default() };
    let colours: Vec<Rgb> = Highlight::ALL.iter().filter_map(|h| h.rgb()).collect();
    let texts: Vec<Rgb> = [Rgb(0, 0, 255), Rgb(255, 0, 0)].into_iter().chain(colours.iter().copied()).collect();
    for &h in &colours {
        let bg = dark.ink_bg(h);
        for &tc in &texts {
            let fg = dark.ink_text(tc, Some(h));
            let k = contrast(fg, bg);
            assert!(k >= 4.5, "text {tc:?} on {h:?}: {fg:?} on {bg:?} has contrast {k:.2}");
        }
    }
}

/// #1252 review: an Automatic underline on a dark highlight (black) draws light on a dark page,
/// like the text it underlines, not the dark grey plain inversion gives white.
#[test]
fn dark_page_underline_on_a_dark_highlight_reads() {
    use wordcraft_doc::props::{Highlight, Underline};
    let cp =
        wordcraft_doc::CharProps { highlight: Some(Highlight::Black), underline: Some(Underline::Thick), size: Some(24.0), ..Default::default() };
    let mut d = Document::from_text("x");
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("HHHH HHHH", cp))];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let items = page_display(&d, &l.pages[0], &DisplayOptions::default());
    let (x0, x1, y, w) = items
        .iter()
        .find_map(|i| match i {
            Draw::Line { x0, x1, y0, width, .. } => Some((*x0, *x1, *y0, *width)),
            _ => None,
        })
        .unwrap();
    assert!(w >= 1.5, "thick underline {w}");
    let (px, py) = (((x0 + x1) / 2.0) as u32, y.round() as u32);
    let dark = render_page(&d, &l.pages[0], 4.0, &RenderOptions { dark: true, ..Default::default() });
    let [r8, g8, b8, _] = dark.pixel(px * 4, py * 4);
    assert!(r8 > 0xC0 && g8 > 0xC0 && b8 > 0xC0, "underline draws light, got {:?}", (r8, g8, b8));
}

/// #2534 review: Automatic text with a legacy shadow (drawn from its outlines) on a black
/// highlight draws light on a dark page, like plain text does, not dark grey on near-black.
#[test]
fn dark_page_shadowed_text_on_a_dark_highlight_reads() {
    use wordcraft_doc::props::Highlight;
    let cp = wordcraft_doc::CharProps { highlight: Some(Highlight::Black), shadow: Some(true), size: Some(24.0), ..Default::default() };
    let mut d = Document::from_text("x");
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("HHHH HHHH", cp))];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let items = page_display(&d, &l.pages[0], &DisplayOptions::default());
    let b = items
        .iter()
        .find_map(|i| match i {
            Draw::Art { path, text_bg, .. } => {
                assert!(text_bg.is_some(), "the run's highlight is the letters' background");
                wordcraft_layout::wordart::path_box(path)
            }
            _ => None,
        })
        .unwrap();
    let dark = render_page(&d, &l.pages[0], 2.0, &RenderOptions { dark: true, ..Default::default() });
    let light = (b.x0 as u32 * 2..b.x1 as u32 * 2)
        .flat_map(|x| (b.y0 as u32 * 2..b.y1 as u32 * 2).map(move |y| (x, y)))
        .filter(|&(x, y)| dark.pixel(x, y)[..3].iter().all(|&v| v > 0xE0))
        .count();
    assert!(light > 200, "letters draw light, {light} light pixels");
}

/// One EMF record: type, size, payload (a multiple of 4 bytes).
fn emf_rec(typ: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = typ.to_le_bytes().to_vec();
    v.extend(((8 + payload.len()) as u32).to_le_bytes());
    v.extend(payload);
    v
}

fn le32(vals: &[i32]) -> Vec<u8> {
    vals.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A 25 mm square EMF (device units 0.04 per 0.01 mm, so 100 device units) with a red rectangle over
/// its right half.
fn red_right_half_emf() -> Vec<u8> {
    let mut h = vec![0u8; 88];
    h[0..4].copy_from_slice(&1u32.to_le_bytes());
    h[4..8].copy_from_slice(&88u32.to_le_bytes());
    h[8..24].copy_from_slice(&le32(&[0, 0, 100, 100]));
    h[24..40].copy_from_slice(&le32(&[0, 0, 2500, 2500]));
    h[40..44].copy_from_slice(&0x464D_4520u32.to_le_bytes());
    h[72..88].copy_from_slice(&le32(&[1000, 1000, 250, 250]));
    let brush = emf_rec(39, &le32(&[1, 0, 0x0000_00FF, 0]));
    let select = emf_rec(37, &le32(&[1]));
    let rect = emf_rec(43, &le32(&[50, 0, 100, 100]));
    let eof = emf_rec(14, &[0; 12]);
    [h, brush, select, rect, eof].concat()
}

#[test]
fn metafile_picture_is_drawn_as_vectors() {
    let mut d = Document::new();
    let media = d.add_media(red_right_half_emf(), "emf");
    let mut p = wordcraft_doc::Paragraph::new();
    let obj = wordcraft_doc::para::InlineObject::Image {
        media,
        w: 70.0,
        h: 70.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let opts = RenderOptions::default();
    let rect = page_display(&d, &l.pages[0], &opts.display)
        .iter()
        .find_map(|i| match i {
            Draw::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("a picture on the page");
    let img = render_page(&d, &l.pages[0], 1.0, &opts);
    let y = (rect.y + rect.h / 2.0) as u32;
    let right = img.pixel((rect.x + rect.w * 0.75) as u32, y);
    let left = img.pixel((rect.x + rect.w * 0.25) as u32, y);
    assert!(right[0] > 200 && right[1] < 60 && right[2] < 60, "right half should be red: {right:?}");
    assert!(left[..3].iter().all(|c| *c > 200), "left half should be paper, not the grey box: {left:?}");
}

/// #1899: a crop that keeps a sliver of the picture (3% here; a header's bar can be 2.7% of a
/// page-sized metafile) stretches that sliver over the frame, as the file says, for vector and
/// bitmap pictures alike. Both pictures are paper on the left half and red on the right.
#[test]
fn a_crop_keeping_a_sliver_fills_the_frame_with_it() {
    let mut png = Vec::new();
    image::RgbaImage::from_fn(100, 100, |x, _| image::Rgba(if x >= 50 { [255, 0, 0, 255] } else { [255; 4] }))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    for (bytes, ext) in [(red_right_half_emf(), "emf"), (png, "png")] {
        // (crop, where in the frame, red?): 47–50% of the source is all paper; 50–53% all red.
        for (crop, at, red) in [([0.47, 0.47, 0.5, 0.5], 0.7, false), ([0.5, 0.47, 0.47, 0.5], 0.3, true)] {
            let mut d = Document::new();
            let media = d.add_media(bytes.clone(), ext);
            let mut p = wordcraft_doc::Paragraph::new();
            let obj =
                wordcraft_doc::para::InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop, ole: None };
            p.insert_object(0, obj, &Default::default()).unwrap();
            d.body = vec![wordcraft_doc::para_block(p)];
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            let opts = RenderOptions::default();
            let rect = page_display(&d, &l.pages[0], &opts.display)
                .iter()
                .find_map(|i| match i {
                    Draw::Image { rect, .. } => Some(*rect),
                    _ => None,
                })
                .expect("a picture on the page");
            let img = render_page(&d, &l.pages[0], 1.0, &opts);
            let px = img.pixel((rect.x + rect.w * at) as u32, (rect.y + rect.h * at) as u32);
            let is_red = px[0] > 200 && px[1] < 60 && px[2] < 60;
            let is_paper = px[..3].iter().all(|c| *c > 200);
            assert!(if red { is_red } else { is_paper }, "{ext} crop {crop:?} at {at}: {px:?}");
        }
    }
}

#[test]
fn cosmetic_metafile_pen_is_one_device_pixel_wide() {
    // MOVETO (0, 50), LINETO (100, 50) with the default cosmetic black pen: a hairline across the middle.
    let mut emf = red_right_half_emf()[..88].to_vec();
    emf.extend([emf_rec(27, &le32(&[0, 50])), emf_rec(54, &le32(&[100, 50])), emf_rec(14, &[0; 12])].concat());
    let mut d = Document::new();
    let media = d.add_media(emf, "emf");
    let mut p = wordcraft_doc::Paragraph::new();
    let obj = wordcraft_doc::para::InlineObject::Image {
        media,
        w: 70.0,
        h: 70.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let opts = RenderOptions::default();
    let rect = page_display(&d, &l.pages[0], &opts.display)
        .iter()
        .find_map(|i| match i {
            Draw::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("a picture on the page");
    let scale = 2.0;
    let img = render_page(&d, &l.pages[0], scale, &opts);
    let (x, y) = (((rect.x + rect.w / 2.0) * scale) as u32, ((rect.y + rect.h / 2.0) * scale) as u32);
    let darkest = (y - 3..=y + 3).map(|y| img.pixel(x, y)[0]).min().unwrap_or(255);
    // A 0.1 pt line would cover a fifth of a pixel (about 204); one device pixel covers at least half of one.
    assert!(darkest < 140, "the hairline should be clearly visible: {darkest}");
}

/// #1764: an EMF picture's text (EXTCREATEFONTINDIRECTW, SELECTOBJECT, SETTEXTCOLOR, EXTTEXTOUTW)
/// is painted, not dropped.
#[test]
fn metafile_text_records_are_painted() {
    let mut font = emf_rec(82, &le32(&[1, -20, 0, 0, 0, 700, 0, 0]));
    let mut name: Vec<u8> = "Arial".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    name.resize(64, 0);
    font.extend(name);
    font[4..8].copy_from_slice(&104u32.to_le_bytes());
    // "WWWW" at (5, 40), top-aligned, no dx array.
    let mut text = emf_rec(84, &le32(&[0, 0, 100, 100, 1, 0, 0, 5, 40, 4, 76, 0, 0, 0, 0, 0, 0]));
    text.extend("WWWW".encode_utf16().flat_map(|u| u.to_le_bytes()));
    text[4..8].copy_from_slice(&84u32.to_le_bytes());
    let mut emf = red_right_half_emf()[..88].to_vec();
    emf.extend([font, emf_rec(37, &le32(&[1])), emf_rec(24, &le32(&[0])), text, emf_rec(14, &[0; 12])].concat());
    let mut d = Document::new();
    let media = d.add_media(emf, "emf");
    let mut p = wordcraft_doc::Paragraph::new();
    let obj = wordcraft_doc::para::InlineObject::Image {
        media,
        w: 70.0,
        h: 70.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let opts = RenderOptions::default();
    let rect = page_display(&d, &l.pages[0], &opts.display)
        .iter()
        .find_map(|i| match i {
            Draw::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("a picture on the page");
    let scale = 2.0;
    let img = render_page(&d, &l.pages[0], scale, &opts);
    let (x0, y0) = ((rect.x * scale) as u32, (rect.y * scale) as u32);
    let (x1, y1) = (((rect.x + rect.w) * scale) as u32, ((rect.y + rect.h) * scale) as u32);
    let dark = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| img.pixel(x, y)[0] < 100).count();
    assert!(dark > 50, "the text is painted inside the picture: {dark} dark pixels");
}

/// An EMF text run whose anchor sits inside the picture but whose font is billions of units
/// tall: each glyph's curves would span billions of device pixels, which the rasteriser
/// subdivides for ever. Glyphs that big (or off the raster) are left out; the page renders.
#[test]
fn metafile_text_with_a_huge_font_is_left_out() {
    let mut font = emf_rec(82, &le32(&[1, -2_000_000_000, 0, 0, 0, 400, 0, 0]));
    let mut name: Vec<u8> = "Arial".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    name.resize(64, 0);
    font.extend(name);
    font[4..8].copy_from_slice(&104u32.to_le_bytes());
    let mut text = emf_rec(84, &le32(&[0, 0, 100, 100, 1, 0, 0, 5, 40, 4, 76, 0, 0, 0, 0, 0, 0]));
    text.extend("WOWS".encode_utf16().flat_map(|u| u.to_le_bytes()));
    text[4..8].copy_from_slice(&84u32.to_le_bytes());
    let mut emf = red_right_half_emf()[..88].to_vec();
    emf.extend([font, emf_rec(37, &le32(&[1])), emf_rec(24, &le32(&[0])), text, emf_rec(14, &[0; 12])].concat());
    let mut d = Document::new();
    let media = d.add_media(emf, "emf");
    let mut p = wordcraft_doc::Paragraph::new();
    let obj = wordcraft_doc::para::InlineObject::Image {
        media,
        w: 70.0,
        h: 70.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let start = std::time::Instant::now();
    let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
    // About 20 ms without the huge glyphs; drawing them ran past 4 minutes without finishing.
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
    assert_eq!(img.width, 612);
    // Leaving them out is logged (once a run, so it may have been by another test already).
    assert!(OVERSIZED_GLYPHS_NOTED.load(std::sync::atomic::Ordering::Relaxed), "the left-out glyphs are logged");
}

/// Glyphs left out for their size are logged the first time only: a page is drawn on every
/// repaint. A run with none left out logs nothing and doesn't use up the warning.
#[test]
fn oversized_glyphs_are_logged_once_a_run() {
    let noted = std::sync::atomic::AtomicBool::new(false);
    assert!(!note_oversized_glyphs(&noted, 0), "nothing left out, nothing logged");
    assert!(!noted.load(std::sync::atomic::Ordering::Relaxed));
    assert!(note_oversized_glyphs(&noted, 4), "the first glyphs left out are logged");
    assert!(!note_oversized_glyphs(&noted, 2), "later ones aren't");
}

#[test]
fn unreadable_metafile_keeps_the_grey_box() {
    let mut d = Document::new();
    // The header alone has no records, so the metafile does not parse.
    let media = d.add_media(red_right_half_emf()[..88].to_vec(), "emf");
    let mut p = wordcraft_doc::Paragraph::new();
    let obj = wordcraft_doc::para::InlineObject::Image {
        media,
        w: 70.0,
        h: 70.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let opts = RenderOptions::default();
    let rect = page_display(&d, &l.pages[0], &opts.display)
        .iter()
        .find_map(|i| match i {
            Draw::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("a picture on the page");
    let img = render_page(&d, &l.pages[0], 1.0, &opts);
    assert_eq!(img.pixel((rect.x + rect.w / 2.0) as u32, (rect.y + rect.h / 2.0) as u32), [0xD0, 0xD0, 0xD0, 255]);
}

#[test]
fn shapes_have_paths() {
    let r = kurbo::Rect::new(0.0, 0.0, 10.0, 10.0);
    for k in [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::FlowchartOr,
        ShapeKind::Star,
        ShapeKind::Heart,
        ShapeKind::Arrow,
        ShapeKind::ArrowLeft,
        ShapeKind::ArrowUp,
        ShapeKind::ArrowDown,
        ShapeKind::Line,
    ] {
        assert!(!shape_path(k, r, None).elements().is_empty());
    }
}

#[test]
fn dashed_outline_has_gaps() {
    use wordcraft_doc::{
        connector::ShapeExtra,
        line_dash::LineDash,
        para::{Anchor, Float, InlineObject, Wrap},
    };
    let image = |dash| {
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 70.0,
            h: 28.0,
            fill: None,
            stroke: Some(Rgb(192, 0, 0)),
            stroke_width: 1.0,
            float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: ShapeExtra { dash, ..Default::default() },
        };
        let mut p = wordcraft_doc::Paragraph::new();
        p.insert_object(0, shape, &Default::default()).unwrap();
        let mut d = Document::new();
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        render_page(&d, &l.pages[0], 4.0, &RenderOptions::default())
    };
    let solid = image(LineDash::Solid);
    let dashed = image(LineDash::Dash);
    assert!(solid.pixel(422, 400)[1] < 80);
    assert_eq!(dashed.pixel(422, 400), [255, 255, 255, 255]);
    assert!(dashed.pixel(406, 400)[1] < 80);
}

/// Distinguish flat, square and round endpoints for both preset lines and connector paths.
#[test]
fn shape_line_caps_reach_the_screen() {
    use wordcraft_doc::{
        connector::{LineCap, ShapeExtra},
        para::{Anchor, Float, InlineObject, Wrap},
    };
    // A preset line uses Draw::Shape; a straight connector uses Draw::Path.
    for kind in [ShapeKind::Line, ShapeKind::StraightConnector] {
        for cap in [LineCap::Flat, LineCap::Square, LineCap::Round] {
            let shape = InlineObject::Shape {
                kind,
                w: 40.0,
                h: 0.0,
                fill: None,
                stroke: Some(Rgb::BLACK),
                stroke_width: 10.0,
                float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
                story: None,
                freeform: None,
                effects: Default::default(),
                extra: ShapeExtra { cap, ..Default::default() },
            };
            let mut p = wordcraft_doc::Paragraph::new();
            p.insert_object(0, shape, &Default::default()).unwrap();
            let mut d = Document::new();
            d.body = vec![wordcraft_doc::para_block(p)];
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            let image = render_page(&d, &l.pages[0], 4.0, &RenderOptions::default());
            let painted = |x, y| image.pixel(x, y)[0] < 80;
            assert!(painted(404, 400), "line interior: {kind:?} {cap:?}");
            assert_eq!(painted(388, 400), cap != LineCap::Flat, "past the endpoint: {kind:?} {cap:?}");
            assert_eq!(painted(384, 384), cap == LineCap::Square, "square corner: {kind:?} {cap:?}");
        }
    }
}

#[test]
fn zero_width_connector_dashes() {
    use wordcraft_doc::{
        connector::ShapeExtra,
        line_dash::LineDash,
        para::{Anchor, Float, InlineObject, Wrap},
    };
    let shape = InlineObject::Shape {
        kind: ShapeKind::ElbowConnector,
        w: 70.0,
        h: 28.0,
        fill: None,
        stroke: Some(Rgb(192, 0, 0)),
        stroke_width: 0.0,
        float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: ShapeExtra { dash: LineDash::Dash, ..Default::default() },
    };
    let mut p = wordcraft_doc::Paragraph::new();
    p.insert_object(0, shape, &Default::default()).unwrap();
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let image = render_page(&d, &l.pages[0], 8.0, &RenderOptions::default());
    // A 0.75 pt stroke has a 3 pt dash followed by a 2.25 pt gap.
    assert!(image.pixel(812, 800)[1] < 80, "inside the first dash");
    assert_eq!(image.pixel(832, 800), [255, 255, 255, 255], "inside the first gap");
}

/// Dash lengths too small to cut the path make kurbo's dash iterator spin without moving along
/// it (and every piece is more stroke to build): such a pattern is drawn solid, as are negative
/// and non-finite ones. A normal dash pattern still dashes.
#[test]
fn hostile_dash_patterns_draw_solid_and_finish() {
    use wordcraft_doc::graphic::PathSeg;
    let line = |dash: Vec<f32>| Draw::Path {
        segs: vec![PathSeg::Move(10.0, 20.0), PathSeg::Line(90.0, 20.0)],
        fill: None,
        stroke: Some(Rgb(0, 0, 0)),
        stroke_width: 2.0,
        dash,
        cap: Default::default(),
    };
    let start = std::time::Instant::now();
    for dash in [vec![1e-30, 1e-30], vec![1e-30], vec![-1.0, 2.0], vec![f32::NAN, 1.0], vec![f32::INFINITY, 1.0], vec![0.0, 0.0]] {
        let img =
            render_items(&Document::new(), &[line(dash.clone())], wordcraft_geom::Rect::new(0.0, 0.0, 100.0, 40.0), 1.0, &RenderOptions::default());
        assert!(img.pixel(50, 20)[3] > 200, "{dash:?} draws solid");
    }
    // Milliseconds drawn solid; drawn as dashes they ran past 4 minutes without finishing.
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
    let dashed =
        render_items(&Document::new(), &[line(vec![10.0, 10.0])], wordcraft_geom::Rect::new(0.0, 0.0, 100.0, 40.0), 1.0, &RenderOptions::default());
    assert!(dashed.pixel(15, 20)[3] > 200 && dashed.pixel(25, 20)[3] == 0, "a normal pattern dashes");
    assert!(safe_dashes([3.0, 1.0], 1000.0).len() == 2);
    assert!(safe_dashes([1e-3, 1e-3], 1000.0).is_empty(), "a million pieces");
    assert!(safe_dashes(std::iter::repeat_n(1.0, 200_000), 1.0).is_empty(), "too many entries");
}

#[test]
fn flowchart_or_spokes() {
    let r = kurbo::Rect::new(0.0, 0.0, 10.0, 10.0);
    let path = shape_path(ShapeKind::FlowchartOr, r, None);
    let elements = path.elements();
    for pair in [
        [kurbo::PathEl::MoveTo((0.0, 5.0).into()), kurbo::PathEl::LineTo((10.0, 5.0).into())],
        [kurbo::PathEl::MoveTo((5.0, 0.0).into()), kurbo::PathEl::LineTo((5.0, 10.0).into())],
    ] {
        assert!(elements.windows(2).any(|w| w == pair), "missing spoke {pair:?}");
    }
}

/// A rounded rectangle laid out and rendered: its corner pixel is painted when the corner is
/// square, left white with the default corner; turned a quarter, the same.
#[test]
fn rendered_rounded_rectangles_follow_their_corner() {
    use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
    let rendered = |corner: Option<u32>, rot: f32| {
        let float = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, rot, ..Default::default() };
        let shape = wordcraft_doc::para::InlineObject::Shape {
            kind: ShapeKind::RoundedRectangle,
            w: 200.0,
            h: 200.0,
            fill: Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(0, 0, 0))),
            stroke: None,
            stroke_width: 0.0,
            float,
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: wordcraft_doc::connector::ShapeExtra { corner, ..Default::default() },
        };
        let mut p = wordcraft_doc::Paragraph::new();
        p.insert_object(0, shape, &Default::default()).unwrap();
        let mut d = Document::new();
        d.body = vec![wordcraft_doc::para_block(p)];
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        render_page(&d, &l.pages[0], 1.0, &RenderOptions::default())
    };
    for rot in [0.0, 90.0] {
        // 3 pt in from the frame's top-left corner, and its middle.
        let (corner, middle) = ((103, 103), (200, 200));
        let square = rendered(Some(0), rot);
        assert_eq!(square.pixel(corner.0, corner.1)[0], 0, "square corner, turned {rot}");
        let rounded = rendered(None, rot);
        assert_eq!(rounded.pixel(corner.0, corner.1)[0], 255, "default corner, turned {rot}");
        assert_eq!(rounded.pixel(middle.0, middle.1)[0], 0, "filled, turned {rot}");
    }
}

/// A rounded rectangle's corner is its file's (`roundRect`'s `adj`), not always 16%.
#[test]
fn rounded_rectangle_corners_follow_their_radius() {
    use kurbo::Shape;
    let r = kurbo::Rect::new(0.0, 0.0, 200.0, 100.0);
    let inside = |corner, pt: (f64, f64)| shape_path(ShapeKind::RoundedRectangle, r, corner).contains(pt.into());
    // Two points inside the top-left corner: 1 pt and 4 pt in from both edges.
    let (tip, near) = ((1.0, 1.0), (4.0, 4.0));
    assert!(inside(Some(0), tip), "square corner");
    assert!(!inside(Some(3_565), tip) && inside(Some(3_565), near), "3.565 pt corner");
    assert!(!inside(None, near), "16 pt corner by default");
    assert!(!inside(Some(u32::MAX), (14.0, 14.0)) && inside(Some(u32::MAX), (100.0, 1.0)), "round ends");
}

/// #611: a line runs as DrawingML's `line` preset does, top-left to bottom-right (Word's
/// files flip it for the other diagonal), not bottom-left to top-right.
#[test]
fn a_line_runs_from_top_left_to_bottom_right() {
    let p = shape_path(ShapeKind::Line, kurbo::Rect::new(10.0, 20.0, 30.0, 60.0), None);
    assert_eq!(p.elements(), &[kurbo::PathEl::MoveTo((10.0, 20.0).into()), kurbo::PathEl::LineTo((30.0, 60.0).into())]);
}

//! Subpixel (LCD) text: coloured edges in the screen's order, the weight and place of greyscale
//! text, and nothing else on the page changed (drawing order, clips, fallbacks).

use super::*;
use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

/// 11 pt at 100 % zoom on a 145 % desktop.
const SCALE: f32 = 96.0 / 72.0 * 1.45;
const TEXT: &str = "Every Thursday evening the old print shop fills with people who make things.";

fn page(text: &str) -> (Document, wordcraft_layout::DocLayout) {
    let d = Document::from_text(text);
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    (d, l)
}

fn opts(hinting: bool, subpixel: Option<Subpixel>) -> RenderOptions {
    RenderOptions { hinting, subpixel, ..Default::default() }
}

/// Ink: how far each channel of each pixel is from white.
fn ink(img: &Rendered) -> u64 {
    img.pixels.as_chunks::<4>().0.iter().map(|p| (0..3).map(|c| 255 - u64::from(p[c])).sum::<u64>()).sum()
}

/// Pixels whose red and blue differ by more than 40: coloured edges.
fn coloured(img: &Rendered) -> usize {
    img.pixels.as_chunks::<4>().0.iter().filter(|p| p[0].abs_diff(p[2]) > 40).count()
}

/// The first and last pixel columns with any ink.
fn ink_columns(img: &Rendered) -> (usize, usize) {
    let w = img.width as usize;
    let cols: Vec<usize> =
        img.pixels.as_chunks::<4>().0.iter().enumerate().filter(|(_, p)| p[..3].iter().any(|c| *c < 250)).map(|(i, _)| i % w.max(1)).collect();
    (cols.iter().copied().min().unwrap_or(0), cols.iter().copied().max().unwrap_or(0))
}

#[test]
fn subpixel_text_has_coloured_edges_and_the_weight_and_place_of_greyscale_text() {
    let (d, l) = page(TEXT);
    let grey = render_page(&d, &l.pages[0], SCALE, &opts(true, None));
    let rgb = render_page(&d, &l.pages[0], SCALE, &opts(true, Some(Subpixel::Rgb)));
    assert_eq!(coloured(&grey), 0);
    assert!(coloured(&rgb) > 200, "coloured edges: {}", coloured(&rgb));
    // The same weight within a few percent, and the text where it was.
    let (a, b) = (ink(&grey), ink(&rgb));
    assert!(b as f64 > a as f64 * 0.9 && (b as f64) < a as f64 * 1.1, "ink: greyscale {a}, subpixel {b}");
    let ((l0, r0), (l1, r1)) = (ink_columns(&grey), ink_columns(&rgb));
    assert!(l0.abs_diff(l1) <= 1 && r0.abs_diff(r1) <= 1, "greyscale {l0}..{r0}, subpixel {l1}..{r1}");
    // Off by default: exports and thumbnails stay greyscale.
    assert_eq!(RenderOptions::default().subpixel, None);
}

#[test]
fn bgr_screens_get_the_rgb_text_with_red_and_blue_swapped() {
    // Black text on white: only which channel takes which subpixel differs.
    let (d, l) = page(TEXT);
    let rgb = render_page(&d, &l.pages[0], SCALE, &opts(true, Some(Subpixel::Rgb)));
    let bgr = render_page(&d, &l.pages[0], SCALE, &opts(true, Some(Subpixel::Bgr)));
    let swapped: Vec<u8> = rgb.pixels.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
    assert!(bgr.pixels == swapped, "BGR is RGB mirrored per pixel");
    assert_ne!(rgb.pixels, bgr.pixels);
}

#[test]
fn text_that_cant_be_drawn_on_the_pixel_grid_stays_greyscale_and_the_page_unchanged() {
    let (d, l) = page(TEXT);
    // Without hinting (mid-zoom), the page is exactly the greyscale one: drawing in sections
    // changes nothing else.
    let plain = render_page(&d, &l.pages[0], SCALE, &opts(false, None));
    let sub = render_page(&d, &l.pages[0], SCALE, &opts(false, Some(Subpixel::Rgb)));
    assert!(plain.pixels == sub.pixels);
    // A rotated or mirrored view: greyscale.
    for view in [
        Affine::translate((300.0, -50.0)) * Affine::rotate(0.3) * Affine::scale(1.9),
        Affine::translate((600.0, 0.0)) * Affine::scale_non_uniform(-1.9, 1.9),
    ] {
        let grey = render_region(&d, &l.pages[0], 600, 300, view, &opts(true, None));
        let sub = render_region(&d, &l.pages[0], 600, 300, view, &opts(true, Some(Subpixel::Rgb)));
        assert!(ink(&grey) > 0 && grey.pixels == sub.pixels, "{view:?}");
    }
}

/// A run of "H"s in Source Sans 3 at 11 pt from x = 10 pt on baseline `y` (page points).
fn run(y: f32, color: Rgb) -> Draw {
    let face = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    let gid = face.glyph_for('H');
    Draw::Glyphs {
        face,
        size: 11.0,
        glyphs: (0..12).map(|i| (gid, 10.0 + i as f32 * 8.0, y)).collect(),
        color,
        alpha: 1.0,
        synth_bold: false,
        synth_italic: false,
        text: "HHHHHHHHHHHH".into(),
        link: None,
        ranges: Vec::new(),
        text_bg: None,
        x_scale: 1.0,
    }
}

fn fill(x: f32, y: f32, w: f32, h: f32, color: Rgb) -> Draw {
    Draw::Fill { rect: wordcraft_geom::Rect::new(x, y, w, h), color, alpha: 1.0, text_bg: false }
}

#[test]
fn text_in_a_faded_layer_is_drawn_greyscale_and_faded() {
    // Editing a header fades the body as one layer. Subpixel text is blended straight onto the
    // pixels, past the layer, so text in it is drawn greyscale, as without subpixel text.
    let (d, l) = page(TEXT);
    let faded = |subpixel| {
        let mut o = opts(true, subpixel);
        o.display.dim_body = true;
        render_page(&d, &l.pages[0], SCALE, &o)
    };
    let (grey, rgb) = (faded(None), faded(Some(Subpixel::Rgb)));
    assert!(ink(&grey) > 0);
    assert_eq!(coloured(&rgb), 0);
    assert!(grey.pixels == rgb.pixels, "faded text differs from greyscale");
}

#[test]
fn scaled_letters_are_drawn_greyscale_at_their_width() {
    // Scaled letters (`w:w`) are stretched across from each pen position; subpixel masks are not.
    let d = Document::new();
    let view = Affine::scale(f64::from(SCALE));
    let mut wide = run(20.0, Rgb::BLACK);
    if let Draw::Glyphs { x_scale, .. } = &mut wide {
        *x_scale = 2.0;
    }
    let grey = render_draws(&d, std::slice::from_ref(&wide), 220, 60, view, &opts(true, None));
    let rgb = render_draws(&d, &[wide], 220, 60, view, &opts(true, Some(Subpixel::Rgb)));
    assert!(ink(&grey) > 0);
    assert!(grey.pixels == rgb.pixels, "scaled letters differ from greyscale");
}

#[test]
fn subpixel_text_keeps_its_place_in_the_drawing_order() {
    let d = Document::new();
    let view = Affine::scale(f64::from(SCALE));
    let sub = opts(true, Some(Subpixel::Rgb));
    let (w, h) = (220, 60);
    // Text under a white fill drawn after it: covered, the page all white.
    let covered = render_draws(&d, &[run(20.0, Rgb::BLACK), fill(0.0, 0.0, 200.0, 40.0, Rgb::WHITE)], w, h, view, &sub);
    assert_eq!(ink(&covered), 0, "the fill drawn after the text covers it");
    // Text on a blue fill drawn before it: on the blue, its edges coloured against it.
    let blue = Rgb(0x20, 0x40, 0xC0);
    let on = render_draws(&d, &[fill(0.0, 0.0, 200.0, 40.0, blue), run(20.0, Rgb::WHITE)], w, h, view, &sub);
    let px = on.pixels.as_chunks::<4>().0;
    assert!(px.iter().any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200), "white text drawn");
    assert!(px.iter().filter(|p| p[..3] == [0x20, 0x40, 0xC0]).count() > 1000, "on the blue fill");
}

#[test]
fn a_shape_drawn_over_text_covers_it() {
    // A path (drawn by vello_cpu, not as a rectangle) over text that waits: the text is blended
    // first, then the path covers it. A cell fill or border over text waits with it, in order.
    let d = Document::new();
    let view = Affine::scale(f64::from(SCALE));
    let (w, h) = (220, 60);
    use wordcraft_doc::graphic::PathSeg;
    let white = Draw::Path {
        segs: vec![PathSeg::Move(0.0, 0.0), PathSeg::Line(200.0, 0.0), PathSeg::Line(200.0, 40.0), PathSeg::Line(0.0, 40.0), PathSeg::Close],
        fill: Some(wordcraft_layout::display::Paint::Solid(Rgb::WHITE, 1.0)),
        stroke: None,
        stroke_width: 0.0,
        dash: Vec::new(),
        cap: Default::default(),
    };
    for (name, over) in [("path", white), ("fill", fill(0.0, 0.0, 200.0, 40.0, Rgb::WHITE))] {
        let img = render_draws(&d, &[run(20.0, Rgb::BLACK), over], w, h, view, &opts(true, Some(Subpixel::Rgb)));
        assert_eq!(ink(&img), 0, "the {name} drawn after the text covers it");
    }
}

#[test]
fn subpixel_text_in_a_clip_stays_inside_it() {
    let d = Document::new();
    let view = Affine::scale(f64::from(SCALE));
    let (w, h) = (220, 60);
    // A clip over the left half of the run, then a fill after it (sections render around it).
    let clip = Draw::Clip {
        rect: wordcraft_geom::Rect::new(0.0, 0.0, 50.0, 40.0),
        items: vec![run(20.0, Rgb::BLACK), fill(60.0, 0.0, 5.0, 5.0, Rgb::BLACK)],
    };
    let after = fill(100.0, 30.0, 5.0, 5.0, Rgb::BLACK);
    for subpixel in [None, Some(Subpixel::Rgb)] {
        let img = render_draws(&d, &[clip.clone(), after.clone()], w, h, view, &opts(true, subpixel));
        let edge = (50.0 * SCALE).round() as usize;
        let inked_right = img.pixels.as_chunks::<4>().0.iter().enumerate().filter(|(i, p)| {
            let (x, y) = (i % w as usize, i / w as usize);
            x > edge && y < (28.0 * SCALE) as usize && p[..3].iter().any(|c| *c < 250)
        });
        assert_eq!(inked_right.count(), 0, "{subpixel:?}: nothing right of the clip but the fill after it");
        assert!(ink(&img) > 0, "{subpixel:?}");
    }
}

#[test]
fn hostile_glyph_runs_never_panic() {
    let d = Document::new();
    let view = Affine::scale(f64::from(SCALE));
    let face = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    let gid = face.glyph_for('H');
    let mut runs = Vec::new();
    for (size, x, y) in
        [(11.0, f32::NAN, 20.0), (11.0, 10.0, f32::INFINITY), (5000.0, 10.0, 20.0), (0.0, 10.0, 20.0), (-3.0, 10.0, 20.0), (11.0, -1e9, 1e9)]
    {
        let Draw::Glyphs { glyphs, size: s, .. } = &mut run(20.0, Rgb::BLACK) else { continue };
        *glyphs = vec![(gid, x, y), (u32::MAX, 10.0, 20.0)];
        *s = size;
        runs.push(Draw::Glyphs {
            face,
            size: *s,
            glyphs: glyphs.clone(),
            color: Rgb::BLACK,
            alpha: 1.0,
            synth_bold: true,
            synth_italic: true,
            text: String::new(),
            link: None,
            ranges: Vec::new(),
            text_bg: None,
            x_scale: 1.0,
        });
    }
    let img = render_draws(&d, &runs, 200, 60, view, &opts(true, Some(Subpixel::Rgb)));
    assert_eq!(img.pixels.len(), 200 * 60 * 4);
}

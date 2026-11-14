use super::*;
use crate::cmd::insert::base64_encode;

/// A noisy blue backdrop with a noisy orange disc in the middle (radius 0.3 of the width).
fn photo(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| {
        let n = ((x.wrapping_mul(7919) ^ y.wrapping_mul(104_729)) % 23) as f32 - 11.0;
        let (dx, dy) = (x as f32 + 0.5 - w as f32 / 2.0, y as f32 + 0.5 - h as f32 / 2.0);
        let c = if (dx * dx + dy * dy).sqrt() < 0.3 * w as f32 { [220.0, 120.0, 40.0] } else { [70.0, 120.0, 210.0] };
        image::Rgba([(c[0] + n) as u8, (c[1] + n) as u8, (c[2] + n) as u8, 255])
    });
    let mut v = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
    v
}

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.picture", &json!({"data": base64_encode(&photo(w, h))})).unwrap();
    s
}

fn bitmap(s: &Session) -> RgbaImage {
    let (_, InlineObject::Image { media, .. }) = selected(s).unwrap() else { panic!("a picture") };
    image::load_from_memory(&s.doc.media.get(&media).cloned().unwrap()).unwrap().to_rgba8()
}

fn at(img: &RgbaImage, fx: f32, fy: f32) -> [u8; 4] {
    img.get_pixel(((fx * img.width() as f32) as u32).min(img.width() - 1), ((fy * img.height() as f32) as u32).min(img.height() - 1)).0
}

#[test]
fn remove_background_clears_around_the_subject_in_one_undo_step() {
    let mut s = session(120, 120);
    let before = bitmap(&s);
    let out = s.run("picture.removeBackground", &json!({})).unwrap();
    let kept = out["kept"].as_f64().unwrap();
    assert!((0.2..0.4).contains(&kept), "about π·0.3² of it: {kept}");
    let img = bitmap(&s);
    assert_eq!(at(&img, 0.05, 0.05)[3], 0, "the backdrop");
    assert_eq!(at(&img, 0.5, 0.5), at(&before, 0.5, 0.5), "the subject as it was");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(bitmap(&s), before);
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(at(&bitmap(&s), 0.05, 0.05)[3], 0);
    s.run("picture.reset", &json!({})).unwrap();
    assert_eq!(bitmap(&s), before, "Reset Picture brings the background back");
}

/// #3344: Remove Background is one layer of the stack, so Color, Transparency and Corrections
/// after it keep the background removed, and it can be refined or discarded after them.
#[test]
fn remove_background_keeps_through_other_adjustments_and_can_be_refined() {
    let mut s = session(120, 120);
    s.run("picture.removeBackground", &json!({})).unwrap();
    s.run("picture.color", &json!({"recolor": "grayscale"})).unwrap();
    let img = bitmap(&s);
    assert_eq!(at(&img, 0.05, 0.05)[3], 0, "still removed after Color");
    let [r, g, b, a] = at(&img, 0.5, 0.5);
    assert!(r == g && g == b && a == 255, "the subject in gray");
    s.run("picture.transparency", &json!({"percent": 50})).unwrap();
    s.run("picture.corrections", &json!({"brightness": 20})).unwrap();
    let img = bitmap(&s);
    assert_eq!(at(&img, 0.05, 0.05)[3], 0);
    assert_eq!(at(&img, 0.5, 0.5)[3], 127, "half transparent");
    // A keep mark on the backdrop brings it back: the removal runs on the bitmap under it all.
    let keep = json!({"marks": [{"keep": true, "points": [[0.05, 0.05], [0.05, 0.3]], "width": 0.05}]});
    s.run("picture.removeBackground", &keep).unwrap();
    let img = bitmap(&s);
    assert_eq!(at(&img, 0.05, 0.1)[3], 127, "kept, with the transparency over it");
    assert_eq!(at(&img, 0.95, 0.95)[3], 0, "the rest of the backdrop still goes");
    // Discard All Changes: the background back, everything else kept.
    s.run("picture.removeBackground", &json!({"discard": true})).unwrap();
    let img = bitmap(&s);
    assert_eq!(at(&img, 0.95, 0.95)[3], 127);
    let [r, g, b, _] = at(&img, 0.95, 0.95);
    assert!(r == g && g == b, "still gray");
    // Undo goes back through each step, and the look comes back with it.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(at(&bitmap(&s), 0.95, 0.95)[3], 0);
    s.run("picture.transparency", &json!({"percent": 0})).unwrap();
    let img = bitmap(&s);
    assert_eq!((at(&img, 0.95, 0.95)[3], at(&img, 0.5, 0.5)[3]), (0, 255), "the removal stays when transparency goes");
}

/// Scripts that pass `tolerance` keep the older fill of the top-left corner's colour.
#[test]
fn remove_background_with_a_tolerance_fills_the_corner_colour() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let ring = RgbaImage::from_fn(60, 60, |x, y| {
        let d = (x as i32 - 30).abs().max((y as i32 - 30).abs());
        if (10..14).contains(&d) { image::Rgba([200, 20, 20, 255]) } else { image::Rgba([255, 255, 255, 255]) }
    });
    let mut png = Vec::new();
    ring.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.run("insert.picture", &json!({"data": base64_encode(&png)})).unwrap();
    s.run("picture.removeBackground", &json!({"tolerance": 30})).unwrap();
    let img = bitmap(&s);
    assert_eq!(img.get_pixel(0, 0).0[3], 0);
    assert_eq!(img.get_pixel(30, 30).0[3], 255, "what the ring encloses stays");
}

#[test]
fn set_transparent_color_clears_one_colour_everywhere() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let img = RgbaImage::from_fn(40, 20, |x, _| if x < 20 { image::Rgba([10, 200, 30, 255]) } else { image::Rgba([12, 196, 33, 255]) });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.run("insert.picture", &json!({"data": base64_encode(&png)})).unwrap();
    // Picked on the picture: that colour and its near neighbours (a JPEG's speckle) go.
    s.run("picture.transparentColor", &json!({"x": 0.1, "y": 0.5})).unwrap();
    assert!(bitmap(&s).pixels().all(|p| p.0[3] == 0));
    s.run("picture.transparentColor", &json!({"color": null})).unwrap();
    assert!(bitmap(&s).pixels().all(|p| p.0[3] == 255), "none: the picture as it was");
    s.run("picture.transparentColor", &json!({"color": "FF0000"})).unwrap();
    assert!(bitmap(&s).pixels().all(|p| p.0[3] == 255), "a colour it doesn't have");
    for junk in [json!({}), json!({"color": 3}), json!({"color": "nope"}), json!({"x": 0.5})] {
        assert!(matches!(s.run("picture.transparentColor", &junk), Err(CmdError::Params(_))), "{junk}");
    }
}

#[test]
fn color_tone_recolor_and_corrections_each_keep_the_rest() {
    let mut s = session(40, 40);
    let mid = |s: &Session| at(&bitmap(s), 0.5, 0.5);
    let orig = mid(&s);
    s.run("picture.color", &json!({"temperature": 11200})).unwrap();
    let warm = mid(&s);
    assert!(warm[0] > orig[0] && warm[2] < orig[2], "warmer: {warm:?} from {orig:?}");
    s.run("picture.color", &json!({"temperature": 4700})).unwrap();
    let cool = mid(&s);
    assert!(cool[0] < orig[0] && cool[2] > orig[2], "cooler: {cool:?}");
    // Saturation keeps the tone; No Recolor keeps both.
    s.run("picture.color", &json!({"saturation": 0})).unwrap();
    s.run("picture.color", &json!({"recolor": "none"})).unwrap();
    let look = current(&s).unwrap();
    assert_eq!((look.color.saturation, look.color.temperature), (0.0, 4700.0));
    s.run("picture.color", &json!({"saturation": 100, "temperature": 6500})).unwrap();
    assert_eq!(mid(&s), orig, "neutral again: the picture as it was");
    // A dark recolour stays in the colour's shades; a light one is lighter.
    s.run("picture.color", &json!({"recolor": "dark", "color": "2060C0"})).unwrap();
    let [r, g, b, _] = mid(&s);
    assert!(b >= g && g >= r, "{r} {g} {b}");
    s.run("picture.color", &json!({"recolor": "light", "color": "2060C0"})).unwrap();
    assert!(mid(&s)[0] > r);
    assert!(matches!(s.run("picture.color", &json!({"recolor": "dark"})), Err(CmdError::Params(_))), "needs a colour");
    assert!(matches!(s.run("picture.color", &json!({"recolor": "purple"})), Err(CmdError::Params(_))));
    // Black and white: the threshold decides how much is white.
    let white = |s: &mut Session, t: f32| {
        s.run("picture.color", &json!({"recolor": "blackAndWhite", "threshold": t})).unwrap();
        bitmap(s).pixels().filter(|p| p.0[0] == 255).count()
    };
    assert!(white(&mut s, 25.0) > white(&mut s, 75.0));
    // Sharpen keeps the brightness.
    s.run("picture.color", &json!({"recolor": "none"})).unwrap();
    s.run("picture.corrections", &json!({"brightness": 30})).unwrap();
    s.run("picture.corrections", &json!({"sharpen": 25})).unwrap();
    assert_eq!(current(&s).unwrap().corrections, Corrections { brightness: 30.0, contrast: 0.0, sharpen: 25.0 });
}

#[test]
fn every_artistic_effect_draws_and_keeps_the_size() {
    for (w, h) in [(48, 32), (1, 1), (1, 300), (300, 2)] {
        let mut s = session(w, h);
        let before = bitmap(&s);
        for e in EFFECTS {
            s.run("picture.effects", &json!({"effect": e})).unwrap_or_else(|err| panic!("{e} on {w}×{h}: {err:?}"));
            let img = bitmap(&s);
            assert_eq!(img.dimensions(), (w, h), "{e}");
            if (w, h) == (48, 32) && !matches!(e, "pixelate") {
                assert_ne!(img, before, "{e} changes the picture");
            }
        }
        s.run("picture.effects", &json!({"effect": "none"})).unwrap();
        assert_eq!(bitmap(&s), before, "None: as it was");
    }
    let mut s = session(20, 20);
    assert!(matches!(s.run("picture.effects", &json!({"effect": "sparkles"})), Err(CmdError::Params(_))));
    // An effect after Remove Background keeps the background removed.
    let mut s = session(120, 120);
    s.run("picture.removeBackground", &json!({})).unwrap();
    for e in ["pencilSketch", "paintBrush", "mosaicBubbles", "glowEdges"] {
        s.run("picture.effects", &json!({"effect": e})).unwrap();
        assert_eq!(at(&bitmap(&s), 0.03, 0.03)[3], 0, "{e}");
    }
}

/// A picture larger than the effects draw is shrunk for them, and stays shrunk.
#[test]
fn artistic_effects_on_a_large_picture_are_bounded() {
    let mut s = session(2000, 1500);
    s.run("picture.effects", &json!({"effect": "paintBrush"})).unwrap();
    let (w, h) = bitmap(&s).dimensions();
    assert!(u64::from(w) * u64::from(h) <= 2_000_000 && (w as f32 / h as f32 - 4.0 / 3.0).abs() < 0.01, "{w}×{h}");
}

#[test]
fn picture_border_keeps_its_parts_and_no_outline_removes_it() {
    let mut s = session(40, 40);
    s.run("picture.border", &json!({"width": 6})).unwrap();
    s.run("picture.border", &json!({"color": "FF0000"})).unwrap();
    assert_eq!(current(&s).unwrap().border, Some((Rgb(255, 0, 0), 6)), "the width kept");
    assert_eq!(bitmap(&s).dimensions(), (52, 52));
    s.run("picture.border", &json!({"color": null})).unwrap();
    assert_eq!(bitmap(&s).dimensions(), (40, 40), "No Outline");
    assert!(matches!(s.run("picture.style", &json!({"style": "sparkly"})), Err(CmdError::Params(_))));
    s.run("picture.style", &json!({"style": "thickFrame"})).unwrap();
    s.run("picture.style", &json!({"style": "none"})).unwrap();
    assert_eq!(bitmap(&s).dimensions(), (40, 40));
}

#[test]
fn reset_picture_and_size_restores_the_original_size() {
    let mut s = session(96, 48);
    s.run("picture.size", &json!({"width": 300})).unwrap();
    s.run("picture.color", &json!({"recolor": "sepia"})).unwrap();
    s.run("picture.reset", &json!({})).unwrap();
    assert_eq!(super::super::select::obj_size(&selected(&s).unwrap().1), (300.0, 150.0), "Reset Picture keeps the size");
    s.run("picture.reset", &json!({"size": true})).unwrap();
    assert_eq!(super::super::select::obj_size(&selected(&s).unwrap().1), (72.0, 36.0), "96 × 48 px at 96 ppi");
}

#[test]
fn previews_draw_a_command_without_running_it() {
    let mut s = session(60, 60);
    s.run("picture.removeBackground", &json!({})).unwrap();
    let thumb = image::load_from_memory(&photo(30, 30)).unwrap().to_rgba8();
    let depth = s.undo_depth();
    let p = preview(&s, &thumb, "picture.color", &json!({"recolor": "grayscale"})).unwrap();
    assert_eq!(p.dimensions(), (30, 30));
    assert_eq!(p.get_pixel(1, 1).0[3], 0, "the removal shrunk onto the thumbnail");
    let [r, g, b, _] = p.get_pixel(15, 15).0;
    assert!(r == g && g == b);
    assert_eq!(s.undo_depth(), depth, "nothing ran");
    assert!(preview(&s, &thumb, "format.bold", &json!({})).is_none());
    assert!(preview(&s, &thumb, "picture.effects", &json!({"effect": "nope"})).is_none());
}

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
    assert_eq!(current(&s).unwrap().border, Some(Border { color: Rgb(255, 0, 0), width: BorderWidth::Px(6), dash: Dash::Solid }), "the width kept");
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

/// The selected picture's frame: its offsets, size and crop.
fn frame_of(s: &Session) -> (f32, f32, f32, f32, [f32; 4]) {
    let (_, InlineObject::Image { w, h, crop, float, .. }) = selected(s).unwrap() else { panic!("a picture") };
    (float.x, float.y, w, h, crop)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

/// Picture Effects reach outside the picture: its frame grows by their room, the picture keeping
/// its own size and place; taking them off gives the frame back. One undo step each.
#[test]
fn picture_effects_grow_the_frame_and_keep_the_picture_in_place() {
    let mut s = session(80, 60);
    s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
    s.run("arrange.bounds", &json!({"x": 100.0, "y": 50.0})).unwrap();
    let (x0, y0, w0, h0, _) = frame_of(&s);
    let plain = bitmap(&s);
    let depth = s.undo_depth();
    s.run("picture.pictureEffects", &json!({"shadow": {"preset": "offsetBottomRight", "distance": 6, "blur": 2}})).unwrap();
    assert_eq!(s.undo_depth(), depth + 1);
    let (x, y, w, h, crop) = frame_of(&s);
    let sh = shown(&s).unwrap();
    assert!(close(sh.w, w0) && close(sh.h, h0), "the picture keeps its size: {sh:?}");
    assert!(w > w0 + 5.0 && h > h0 + 5.0 && crop == [0.0; 4], "the frame holds the shadow: {w} x {h}");
    assert!(close(x + sh.room[0], x0) && close(y + sh.room[1], y0), "and the picture its place");
    assert_eq!(bitmap(&s).dimensions().0 as f32, 80.0 + (sh.room[0] + sh.room[2]) * 80.0 / w0);
    s.run("picture.pictureEffects", &json!({"shadow": null})).unwrap();
    let (x, y, w, h, crop) = frame_of(&s);
    assert!(close(x, x0) && close(y, y0) && close(w, w0) && close(h, h0) && crop == [0.0; 4]);
    assert_eq!(bitmap(&s), plain, "the picture as it was");
    s.run("edit.undo", &json!({})).unwrap();
    assert!(frame_of(&s).2 > w0 + 5.0, "undo brings the shadow back");
    assert!(shown(&s).is_some_and(|sh| close(sh.w, w0)), "with its look");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(frame_of(&s).2, w0);
}

/// A turned picture keeps its centre where it was as its frame grows on one side.
#[test]
fn a_turned_picture_keeps_its_centre_as_the_frame_grows() {
    let mut s = session(80, 40);
    s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
    s.run("arrange.bounds", &json!({"x": 100.0, "y": 100.0})).unwrap();
    s.run("arrange.rotation", &json!({"degrees": 90.0})).unwrap();
    let centre = |s: &Session| {
        let (x, y, w, h, _) = frame_of(s);
        let sh = shown(s).unwrap();
        // The picture's centre from the frame's, turned 90° clockwise: (dx, dy) → (−dy, dx).
        let (dx, dy) = ((sh.room[0] - sh.room[2]) / 2.0, (sh.room[1] - sh.room[3]) / 2.0);
        (x + w / 2.0 - dy, y + h / 2.0 + dx)
    };
    let before = centre(&s);
    s.run("picture.pictureEffects", &json!({"reflection": "fullTouching"})).unwrap();
    let after = centre(&s);
    assert!(close(before.0, after.0) && close(before.1, after.1), "{before:?} → {after:?}");
}

/// What a look draws round the picture goes round what the crop shows; cropping again draws it
/// round the new crop, and without it the crop is the picture's own again.
#[test]
fn borders_and_effects_go_round_what_the_crop_shows() {
    let mut s = session(100, 100);
    s.run("picture.crop", &json!({"left": 0.2, "right": 0.2, "width": 45.0, "height": 75.0})).unwrap();
    s.run("picture.border", &json!({"width": 4})).unwrap();
    assert_eq!(bitmap(&s).dimensions(), (68, 108), "the visible part and its border");
    let sh = shown(&s).unwrap();
    assert_eq!(sh.crop, [0.2, 0.0, 0.2, 0.0]);
    assert!(close(sh.w, 45.0) && close(sh.h, 75.0));
    assert_eq!(frame_of(&s).4, [0.0; 4], "the bitmap holds only what shows");
    s.run("picture.crop", &json!({"left": 0.1, "right": 0.1, "width": 60.0, "height": 75.0})).unwrap();
    assert_eq!(bitmap(&s).dimensions(), (88, 108), "drawn again round the new crop");
    assert_eq!(shown(&s).unwrap().crop, [0.1, 0.0, 0.1, 0.0]);
    s.run("picture.border", &json!({"color": null})).unwrap();
    let (_, _, w, h, crop) = frame_of(&s);
    assert_eq!(crop, [0.1, 0.0, 0.1, 0.0]);
    assert!(close(w, 60.0) && close(h, 75.0));
    assert_eq!(bitmap(&s).dimensions(), (100, 100));
}

/// Size under a look is the picture's own, as Word's is; Reset Picture gives the frame back first.
#[test]
fn size_and_reset_under_effects_are_the_pictures_own() {
    let mut s = session(96, 48);
    let plain = bitmap(&s);
    s.run("picture.pictureEffects", &json!({"glow": 6})).unwrap();
    s.run("picture.size", &json!({"width": 144.0})).unwrap();
    let sh = shown(&s).unwrap();
    assert!(close(sh.w, 144.0) && close(sh.h, 72.0), "{sh:?}");
    assert!(frame_of(&s).2 > 150.0, "the frame holds the glow too");
    s.run("picture.reset", &json!({})).unwrap();
    let (_, _, w, h, _) = frame_of(&s);
    assert!(close(w, 144.0) && close(h, 72.0), "Reset Picture keeps the size: {w} x {h}");
    assert_eq!(bitmap(&s), plain);
    s.run("picture.pictureEffects", &json!({"softEdge": 4})).unwrap();
    s.run("picture.reset", &json!({"size": true})).unwrap();
    let (_, _, w, h, _) = frame_of(&s);
    assert!(close(w, 72.0) && close(h, 36.0));
}

/// A style brings its effects, Picture Effects change them one by one, and a style picked again
/// replaces them; every gallery style draws.
#[test]
fn a_style_brings_its_effects_and_picture_effects_change_them() {
    let mut s = session(60, 40);
    s.run("picture.style", &json!({"style": "dropShadow"})).unwrap();
    let l = current(&s).unwrap();
    assert!(l.effects.shadow.is_some() && l.style.as_deref() == Some("dropShadow"));
    s.run("picture.pictureEffects", &json!({"shadow": null, "reflection": "tightTouching"})).unwrap();
    let l = current(&s).unwrap();
    assert!(l.effects.shadow.is_none() && l.effects.reflection.is_some() && l.style.is_some());
    s.run("picture.style", &json!({"style": "thinBlackFrame"})).unwrap();
    assert!(current(&s).unwrap().effects.is_empty(), "a style replaces the effects");
    for (id, _) in styles::gallery() {
        s.run("picture.style", &json!({"style": id})).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(bitmap(&s).pixels().any(|p| p.0[3] > 0), "{id}");
    }
    s.run("picture.style", &json!({"style": "none"})).unwrap();
    assert_eq!(bitmap(&s).dimensions(), (60, 40));
    assert!(close(frame_of(&s).2, 45.0));
}

/// Effects are one layer of the stack: Color after a shadow keeps it, and a shadow after Remove
/// Background is the subject's.
#[test]
fn effects_keep_through_adjustments_and_follow_the_removed_background() {
    let mut s = session(120, 120);
    s.run("picture.removeBackground", &json!({})).unwrap();
    s.run("picture.pictureEffects", &json!({"shadow": {"preset": "offsetBottomRight", "transparency": 0, "blur": 1}})).unwrap();
    s.run("picture.color", &json!({"recolor": "grayscale"})).unwrap();
    let l = current(&s).unwrap();
    assert!(l.cutout.is_some() && l.effects.shadow.is_some(), "both kept");
    let img = bitmap(&s);
    let (rx, ry) = (l.room[0], l.room[1]);
    assert_eq!(img.get_pixel(rx + 2, ry + 2).0[3], 0, "the removed backdrop casts no shadow");
    assert!(img.get_pixel(rx + 116, ry + 116).0[3] < 30, "nor at the far corner");
    let [r, g, b, a] = img.get_pixel(rx + 60, ry + 60).0;
    assert!(r == g && g == b && a == 255, "the subject in gray");
}

/// Picture Border's Weight is in points and its Dashes leave gaps.
#[test]
fn border_weight_in_points_and_dashes() {
    let mut s = session(80, 80);
    s.run("picture.border", &json!({"color": "FF0000", "weight": 3})).unwrap();
    assert_eq!(bitmap(&s).dimensions(), (88, 88), "3 pt at 80 px for 60 pt");
    s.run("picture.border", &json!({"dash": "dash"})).unwrap();
    assert_eq!(current(&s).unwrap().border, Some(Border { color: Rgb(255, 0, 0), width: BorderWidth::Pt(3.0), dash: Dash::Dash }));
    let img = bitmap(&s);
    let top: Vec<u8> = (0..88).map(|x| img.get_pixel(x, 1).0[3]).collect();
    assert!(top.contains(&0) && top.contains(&255), "{top:?}");
    assert!(matches!(s.run("picture.border", &json!({"dash": "wavy"})), Err(CmdError::Params(_))));
}

/// Compress Pictures: Delete cropped areas cuts away what the crop hides, a resolution is worked
/// out at each picture's size, and All Pictures goes through the document.
#[test]
fn compress_deletes_cropped_areas_and_can_do_every_picture() {
    let mut s = session(200, 100);
    s.run("picture.crop", &json!({"left": 0.5})).unwrap();
    let out = s.run("picture.compress", &json!({"deleteCropped": true, "ppi": 0})).unwrap();
    assert_eq!(out["compressed"], 1);
    assert_eq!(bitmap(&s).dimensions(), (100, 100));
    assert_eq!(frame_of(&s).4, [0.0; 4]);
    assert_eq!(s.run("picture.compress", &json!({"deleteCropped": true, "ppi": 0})).unwrap()["compressed"], 0, "nothing left to cut");
    s.run("picture.reset", &json!({})).unwrap();
    assert_eq!(bitmap(&s).dimensions(), (200, 100), "Reset Picture still finds the original");
    // A second picture: 400 × 200 px at 300 × 150 pt. At 48 ppi it needs 200 px; the first, 150
    // pt across, needs 100 px of its 200.
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.picture", &json!({"data": base64_encode(&photo(400, 200))})).unwrap();
    let out = s.run("picture.compress", &json!({"ppi": 48, "all": true})).unwrap();
    assert_eq!(out["compressed"], 2);
    assert_eq!(bitmap(&s).dimensions(), (200, 100));
    s.run("picture.compress", &json!({"maxPixels": 32})).unwrap();
    assert_eq!(bitmap(&s).dimensions(), (32, 16), "maxPixels as scripts have used it");
}

/// Compress Pictures resizes the whole source bitmap, so its limit counts what the crop hides.
#[test]
fn compression_preserves_the_resolution_of_the_visible_crop() {
    assert_eq!(crate::cmd::objects::compression_pixels(72.0, 72.0, [0.0; 4], 330.0), 330);
    assert_eq!(
        crate::cmd::objects::compression_pixels(72.0, 72.0, [0.45; 4], 330.0),
        3300,
        "one inch showing a tenth of the source needs 3300 source pixels"
    );
    assert_eq!(
        crate::cmd::objects::compression_pixels(72.0, 144.0, [0.45, 0.0, 0.45, 0.0], 330.0),
        3300,
        "the source's longer axis can differ from the display's"
    );
    assert_eq!(crate::cmd::objects::compression_pixels(144.0, 72.0, [0.0, 0.45, 0.0, 0.45], 220.0), 2200);
    assert_eq!(crate::cmd::objects::compression_pixels(1.0, 1.0, [0.0; 4], 96.0), 32);
    assert_eq!(crate::cmd::objects::compression_pixels(4000.0, 4000.0, [0.95; 4], 330.0), 20000);
}

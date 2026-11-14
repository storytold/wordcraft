use serde_json::json;
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Document, Pos};

use super::*;
use crate::WordApp;
use crate::table_resize::tests::{H, button, screen, steps};

/// A noisy blue backdrop with an orange disc in the middle.
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

fn picture(rot: f32) -> InlineObject {
    InlineObject::Image {
        media: "m".into(),
        w: 200.0,
        h: 200.0,
        alt: String::new(),
        float: wordcraft_doc::para::Float { rot, ..Default::default() },
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    }
}

/// An app whose first paragraph holds a 200 pt photo, selected, on Picture Format.
fn app_with_photo(rot: f32) -> WordApp {
    let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    a.session.doc.media.insert("m".into(), std::sync::Arc::new(photo(100, 100)));
    a.session.doc.insert_object(&Pos::body(0, 0), picture(rot), &Default::default()).unwrap();
    a.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, OBJ.len_utf8())})).unwrap();
    a.run("ui.tab", json!({"tab": "Picture Format"})).unwrap();
    a
}

fn bitmap(app: &WordApp) -> RgbaImage {
    let Some((_, InlineObject::Image { media, .. })) = crate::objects::selected(app) else { panic!("a picture") };
    image::load_from_memory(app.session.doc.media.get(media).unwrap()).unwrap().to_rgba8()
}

#[test]
fn remove_background_shows_its_tab_and_keep_changes_is_one_undo_step() {
    let mut app = app_with_photo(0.0);
    assert!(crate::widgets::enabled(&app, "ui.removeBackground"));
    assert!(!crate::widgets::enabled(&app, "ui.keepBackgroundChanges"), "only in the mode");
    app.run("ui.removeBackground", json!({})).unwrap();
    assert!(active(&app).is_some());
    assert_eq!(app.ui.tab, TAB);
    // The pens: one at a time, and again puts it down.
    app.run("ui.markAreasToKeep", json!({})).unwrap();
    assert_eq!(pen(&app), Some(true));
    app.run("ui.markAreasToRemove", json!({})).unwrap();
    assert_eq!(pen(&app), Some(false));
    app.run("ui.markAreasToRemove", json!({})).unwrap();
    assert_eq!(pen(&app), None);
    let depth = app.session.undo_depth();
    app.run("ui.keepBackgroundChanges", json!({})).unwrap();
    assert!(active(&app).is_none());
    assert_eq!(app.ui.tab, "Picture Format", "back to the tab it came from");
    assert_eq!(app.session.undo_depth(), depth + 1, "one undo step");
    let img = bitmap(&app);
    assert_eq!(img.get_pixel(2, 2).0[3], 0, "the backdrop removed");
    assert_eq!(img.get_pixel(50, 50).0[3], 255, "the subject kept");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(bitmap(&app).get_pixel(2, 2).0[3], 255);
}

#[test]
fn discard_all_changes_leaves_the_picture_or_takes_its_removal_away() {
    let mut app = app_with_photo(0.0);
    let before = bitmap(&app);
    let depth = app.session.undo_depth();
    app.run("ui.removeBackground", json!({})).unwrap();
    app.run("ui.discardBackgroundChanges", json!({})).unwrap();
    assert!(active(&app).is_none());
    assert_eq!((bitmap(&app), app.session.undo_depth()), (before.clone(), depth), "nothing changed");
    // A picture that has a removal: coming back shows its marks; discarding brings the
    // background back.
    let marks = json!([{"keep": false, "points": [[0.5, 0.5]], "width": 0.05}]);
    app.run("picture.removeBackground", json!({"marks": marks})).unwrap();
    app.run("ui.removeBackground", json!({})).unwrap();
    assert_eq!(app.canvas.bg_removal.as_ref().map(|r| r.marks.len()), Some(1));
    app.run("ui.discardBackgroundChanges", json!({})).unwrap();
    assert_eq!(bitmap(&app), before);
}

#[test]
fn the_mode_ends_unapplied_when_the_picture_is_no_longer_selected() {
    let mut app = app_with_photo(0.0);
    app.run("ui.removeBackground", json!({})).unwrap();
    let depth = app.session.undo_depth();
    app.run("caret.set", json!({"pos": Pos::body(0, 0)})).unwrap();
    sync(&mut app);
    assert!(app.canvas.bg_removal.is_none());
    assert_ne!(app.ui.tab, TAB);
    assert_eq!(app.session.undo_depth(), depth);
    // Turned pictures aren't marked on the canvas.
    let mut app = app_with_photo(30.0);
    assert!(!crate::widgets::enabled(&app, "ui.removeBackground"));
    assert!(app.run("ui.removeBackground", json!({})).is_err());
    assert!(app.run("ui.markAreasToKeep", json!({})).is_err());
}

#[test]
fn points_on_screen_and_on_the_picture_agree_through_crop_and_flips() {
    let f = Rect::from_min_max(pos2(100.0, 50.0), pos2(300.0, 150.0));
    let crop = [0.1, 0.2, 0.3, 0.0];
    for flip in [(false, false), (true, false), (false, true), (true, true)] {
        for at in [pos2(100.0, 50.0), pos2(300.0, 150.0), pos2(180.0, 77.0)] {
            let p = to_source(f, crop, flip, at);
            assert!((0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1]), "{p:?}");
            assert!(from_source(f, crop, flip, p).distance(at) < 1e-3, "{flip:?} {at:?}");
        }
    }
    // The frame's top-left is the crop's top-left, or its right when flipped.
    assert_eq!(to_source(f, crop, (false, false), f.min), [0.1, 0.2]);
    let p = to_source(f, crop, (true, false), f.min);
    assert!((p[0] - 0.7).abs() < 1e-6 && (p[1] - 0.2).abs() < 1e-6, "{p:?}");
}

#[test]
fn the_preview_tints_what_will_be_removed() {
    let thumb = image::load_from_memory(&photo(60, 60)).unwrap().to_rgba8();
    let tint = Color32::from_rgb(0xB4, 0x2C, 0xC8);
    let p = preview_image(&thumb, &[], tint);
    let corner = p.get_pixel(1, 1).0;
    assert!(corner[0] > corner[1] && corner[2] > corner[1], "tinted: {corner:?}");
    assert_eq!(p.get_pixel(30, 30), thumb.get_pixel(30, 30), "the subject as it is");
}

/// The whole window with the photo selected and Background Removal on.
fn removal_window() -> H {
    let mut a = app_with_photo(0.0);
    a.run("ui.removeBackground", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    h
}

fn drag(h: &mut H, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.step();
    button(h, from, true);
    h.step();
    for i in 1..=6 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        h.step();
    }
    button(h, to, false);
    steps(h, 2);
}

/// A real mouse drag with Mark Areas to Remove across the subject's middle adds a mark; Keep
/// Changes clears what it covers.
#[test]
fn a_mark_drawn_with_the_mouse_is_kept_with_the_changes() {
    let mut h = removal_window();
    h.state_mut().run("ui.markAreasToRemove", json!({})).unwrap();
    let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).expect("laid out").rect;
    let from = screen(&mut h, r.x + r.w * 0.3, r.y + r.h * 0.5);
    let to = screen(&mut h, r.x + r.w * 0.7, r.y + r.h * 0.5);
    let depth = h.state().session.undo_depth();
    drag(&mut h, from, to);
    let marks = h.state().canvas.bg_removal.as_ref().map(|r| r.marks.clone()).unwrap_or_default();
    assert_eq!(marks.len(), 1, "one mark");
    assert!(!marks[0].keep && marks[0].points.len() >= 3, "{marks:?}");
    assert_eq!(h.state().session.undo_depth(), depth, "marking changes nothing yet");
    // Enter: Keep Changes.
    h.key_press(egui::Key::Enter);
    steps(&mut h, 2);
    assert!(active(h.state()).is_none());
    assert_eq!(h.state().session.undo_depth(), depth + 1);
    let img = bitmap(h.state());
    assert_eq!(img.get_pixel(50, 50).0[3], 0, "the subject's middle, marked, is removed");
    assert_eq!(img.get_pixel(50, 25).0[3], 255, "the rest of the subject stays");
}

/// Set Transparent Color: the next click on the picture clears the colour there.
#[test]
fn set_transparent_color_clears_the_colour_clicked() {
    let mut h = removal_window();
    h.state_mut().run("ui.discardBackgroundChanges", json!({})).unwrap();
    h.state_mut().run("ui.setTransparentColor", json!({})).unwrap();
    steps(&mut h, 2);
    let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).expect("laid out").rect;
    let at = screen(&mut h, r.x + 3.0, r.y + 3.0);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    button(&h, at, true);
    h.step();
    button(&h, at, false);
    steps(&mut h, 2);
    assert!(h.state().canvas.pick_color.is_none());
    let img = bitmap(h.state());
    assert_eq!(img.get_pixel(1, 1).0[3], 0, "the backdrop's colour");
    assert_eq!(img.get_pixel(50, 50).0[3], 255, "not the subject's");
}

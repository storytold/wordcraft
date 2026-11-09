//! Draw › Edit › Format Background (`design.pageFill`): every fill type, one undo step each, and
//! hostile values refused or clamped.

use serde_json::json;

use super::{run, s, tiny_png};

#[test]
fn each_fill_is_one_undo_step_and_replaces_the_last() {
    let mut s = s();
    run(&mut s, "design.pageFill", json!({"fill": "solid", "color": "123456"}));
    assert_eq!(s.doc.settings.page_color, Some(wordcraft_doc::Rgb(0x12, 0x34, 0x56)));
    assert!(s.doc.settings.page_picture.is_none() && s.doc.settings.page_gradient.is_none() && s.doc.settings.page_pattern.is_none());

    run(&mut s, "design.pageFill", json!({"fill": "gradient", "start": "FF0000", "end": "0000FF", "angle": 45, "transparency": 25}));
    let g = s.doc.settings.page_gradient.as_ref().expect("gradient");
    assert_eq!(g.stops.first().map(|s| (s.pos, s.color, s.transparency)), Some((0.0, wordcraft_doc::Rgb(0xFF, 0, 0), 25.0)));
    assert_eq!(g.stops.last().map(|s| (s.pos, s.color, s.transparency)), Some((100.0, wordcraft_doc::Rgb(0, 0, 0xFF), 25.0)));
    assert!(matches!(g.kind, wordcraft_doc::fill::GradientKind::Linear { angle, .. } if (angle - 45.0).abs() < 0.01));
    assert_eq!(s.doc.settings.page_color, Some(wordcraft_doc::Rgb(0xFF, 0, 0)), "an older reader shows the first stop");
    assert!(s.doc.settings.page_pattern.is_none() && s.doc.settings.page_picture.is_none());

    run(&mut s, "design.pageFill", json!({"fill": "pattern", "preset": "dkDnDiag", "fore": "AA0000", "back": "FFFFEE", "transparency": 10}));
    let p = s.doc.settings.page_pattern.as_ref().expect("pattern");
    assert_eq!(
        (p.preset.as_str(), p.fore, p.back, p.transparency),
        ("dkDnDiag", wordcraft_doc::Rgb(0xAA, 0, 0), wordcraft_doc::Rgb(0xFF, 0xFF, 0xEE), 10.0)
    );
    assert!(s.doc.settings.page_gradient.is_none() && s.doc.settings.page_picture.is_none());

    let png = wordcraft_engine_png();
    run(&mut s, "design.pageFill", json!({"fill": "picture", "data": png, "tile": true, "transparency": 30}));
    let pic = s.doc.settings.page_picture.as_ref().expect("picture");
    assert_eq!((pic.fill_type.as_str(), pic.transparency), ("tile", 30.0));
    assert!(s.doc.media.contains_key(&pic.media));
    assert!(s.doc.settings.page_pattern.is_none() && s.doc.settings.page_gradient.is_none());

    // One step each, newest first.
    assert!(s.undo(), "picture undone");
    assert!(s.doc.settings.page_picture.is_none() && s.doc.settings.page_pattern.is_some());
    assert!(s.undo(), "pattern undone");
    assert!(s.doc.settings.page_pattern.is_none() && s.doc.settings.page_gradient.is_some());
    assert!(s.undo(), "gradient undone");
    assert!(s.doc.settings.page_gradient.is_none() && s.doc.settings.page_color == Some(wordcraft_doc::Rgb(0x12, 0x34, 0x56)));
    assert!(s.undo(), "solid undone");
    assert!(s.doc.settings.page_color.is_none(), "back to no fill");
    assert!(s.redo());
    assert_eq!(s.doc.settings.page_color, Some(wordcraft_doc::Rgb(0x12, 0x34, 0x56)));
}

/// The app's own PNG, base64, as a command parameter.
fn wordcraft_engine_png() -> String {
    crate::cmd::insert::base64_encode(&tiny_png())
}

#[test]
fn no_fill_clears_every_kind() {
    let mut s = s();
    run(&mut s, "design.pageFill", json!({"fill": "pattern", "preset": "smGrid"}));
    run(&mut s, "design.pageFill", json!({"fill": "none"}));
    assert!(s.doc.settings.page_color.is_none());
    assert!(s.doc.settings.page_picture.is_none());
    assert!(s.doc.settings.page_gradient.is_none());
    assert!(s.doc.settings.page_pattern.is_none());
}

#[test]
fn multi_stop_gradients_are_sorted_and_keep_their_own_transparency() {
    let mut s = s();
    run(
        &mut s,
        "design.pageFill",
        json!({"fill": "gradient", "angle": 400, "stops": [
            {"pos": 100, "color": "0000FF"},
            {"pos": 0, "color": "FF0000", "transparency": 20},
            {"pos": 50, "color": "00FF00"},
        ]}),
    );
    let g = s.doc.settings.page_gradient.as_ref().expect("gradient");
    assert_eq!(g.stops.iter().map(|s| s.pos).collect::<Vec<_>>(), [0.0, 50.0, 100.0]);
    assert_eq!(g.stops.first().map(|s| s.transparency), Some(20.0));
    assert!(matches!(g.kind, wordcraft_doc::fill::GradientKind::Linear { angle, .. } if (angle - 40.0).abs() < 0.01));
}

/// A picture fill from an existing `media` entry checks that the bytes are a picture: a DOCX
/// picture relationship can hold anything, and a refusal must leave the old fill in place.
#[test]
fn a_picture_fill_refuses_media_that_is_not_a_picture() {
    let mut s = s();
    s.doc.media.insert("junk.png".into(), std::sync::Arc::new(b"not an image".to_vec()));
    let before = s.doc.settings.clone();
    assert!(s.run("design.pageFill", &json!({"fill": "picture", "media": "junk.png"})).is_err(), "junk bytes are refused");
    assert_eq!(s.doc.settings, before, "the previous fill is kept");
    // A real picture already in the document is still accepted by key.
    s.doc.media.insert("real.png".into(), std::sync::Arc::new(tiny_png()));
    run(&mut s, "design.pageFill", json!({"fill": "picture", "media": "real.png"}));
    assert_eq!(s.doc.settings.page_picture.as_ref().map(|p| p.media.as_str()), Some("real.png"));
}

#[test]
fn hostile_values_are_refused_or_clamped() {
    let mut s = s();
    let before = s.doc.settings.clone();
    for bad in [
        json!({"fill": "gradient", "stops": [{"pos": "x", "color": "FF0000"}, {"pos": 100, "color": "0000FF"}]}),
        json!({"fill": "gradient", "stops": [{"pos": 0, "color": "not a colour"}, {"pos": 100, "color": "0000FF"}]}),
        json!({"fill": "gradient", "stops": [{"pos": 0, "color": "FF0000"}]}),
        json!({"fill": "pattern", "preset": "noSuchPattern"}),
        json!({"fill": "picture", "media": "missing.png"}),
        json!({"fill": "picture", "data": "bG9s"}), // "lol": not an image
        json!({"fill": "sparkle"}),
        json!({"fill": "solid"}),
    ] {
        assert!(s.run("design.pageFill", &bad).is_err(), "{bad}");
        assert_eq!(s.doc.settings, before, "{bad} changed the document");
    }
    // Out of range: clamped, not refused.
    run(&mut s, "design.pageFill", json!({"fill": "pattern", "preset": "pct20", "transparency": 500}));
    assert_eq!(s.doc.settings.page_pattern.as_ref().map(|p| p.transparency), Some(100.0));
    run(&mut s, "design.pageFill", json!({"fill": "gradient", "start": "000000", "end": "FFFFFF", "angle": -90, "transparency": -5}));
    let g = s.doc.settings.page_gradient.as_ref().expect("gradient");
    assert!(g.stops.iter().all(|s| s.transparency == 0.0));
    assert!(matches!(g.kind, wordcraft_doc::fill::GradientKind::Linear { angle, .. } if (angle - 270.0).abs() < 0.01));
    // Positions outside 0–100 are clamped; a stop transparency that isn't a number is refused.
    let before = s.doc.settings.clone();
    assert!(
        s.run(
            "design.pageFill",
            &json!({"fill": "gradient", "stops": [{"pos": 0, "color": "FF0000", "transparency": "x"}, {"pos": 100, "color": "00FF00"}]})
        )
        .is_err()
    );
    assert_eq!(s.doc.settings, before);
    run(&mut s, "design.pageFill", json!({"fill": "gradient", "stops": [{"pos": -20, "color": "FF0000"}, {"pos": 500, "color": "00FF00"}]}));
    let g = s.doc.settings.page_gradient.as_ref().expect("gradient");
    assert_eq!(g.stops.iter().map(|s| s.pos).collect::<Vec<_>>(), [0.0, 100.0]);
}

use std::sync::Arc;

use wordcraft_doc::para::{InlineObject, ShapeKind};
use wordcraft_doc::{Document, Pos};

use super::*;

/// #4764: Ctrl+Shift drags the selected object into an aligned copy, without changing
/// the original or clipboard, and the whole drop is one undo step.
#[test]
fn ctrl_shift_drag_copies_along_the_dominant_axis() {
    use egui::{Event, Modifiers, PointerButton};
    for (dx, dy, want_x, want_y) in [(150.0, 35.0, 350.0, 300.0), (25.0, 120.0, 200.0, 420.0)] {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        app.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 60})).unwrap();
        app.run("arrange.bounds", json!({"page": 0, "x": 200, "y": 300})).unwrap();
        app.session.clipboard_text = "untouched clipboard".into();
        let before = app.session.doc.clone();
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 1100.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(6);
        let from = crate::canvas::page_to_screen(h.state_mut(), 0, 250.0, 330.0).unwrap();
        let to = crate::canvas::page_to_screen(h.state_mut(), 0, 250.0 + dx, 330.0 + dy).unwrap();
        let mods = Modifiers { shift: true, ..Modifiers::COMMAND };
        h.event(Event::ModifiersChanged(mods));
        h.event(Event::PointerMoved(from));
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
        h.run_steps(1);
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.run_steps(1);
        }
        assert!(h.state().canvas.object_gesture().is_some(), "Ctrl+Shift starts a copy drag");
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
        h.run_steps(3);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(1);
        assert!(h.state().canvas.object_gesture().is_none());
        assert_eq!(h.state().session.clipboard_text, "untouched clipboard");
        let (pos, _) = selected(h.state()).expect("the duplicate is selected");
        let layout = h.state_mut().session.layout();
        let copy = layout.object(&pos, 0).unwrap();
        assert!((copy.rect.x - want_x).abs() < 0.01 && (copy.rect.y - want_y).abs() < 0.01, "aligned drop: {:?}", copy.rect);
        assert!(layout.find_object(0, |o| (o.rect.x - 200.0).abs() < 0.01 && (o.rect.y - 300.0).abs() < 0.01).is_some());
        let after = h.state().session.doc.clone();
        assert_ne!(after, before);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(h.state().session.doc, before);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(h.state().session.doc, after);
    }
}

/// Guides can snap an aligned copy along its free axis, and live Shift changes or Escape
/// do not leave a stale constraint or commit a cancelled copy.
#[test]
fn ctrl_shift_drag_keeps_its_axis_with_guides_and_live_modifiers() {
    use egui::{Event, Modifiers, PointerButton};
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), Default::default());
    app.run("view.commentsPane", json!({"value": false})).unwrap();
    app.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 60})).unwrap();
    app.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
    app.run("caret.docEnd", json!({})).unwrap();
    app.run("insert.shape", json!({"kind": "rectangle", "width": 60, "height": 60})).unwrap();
    app.run("arrange.bounds", json!({"page": 0, "x": 450, "y": 252})).unwrap();
    app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 3)})).unwrap();
    let before = app.session.doc.clone();
    let depth = app.session.undo_depth();
    let mut h = rotation_window(app);
    let centre = h.state_mut().session.layout().pages[0].w / 2.0;
    let from = crate::canvas::page_to_screen(h.state_mut(), 0, 150.0, 280.0).unwrap();
    let to = crate::canvas::page_to_screen(h.state_mut(), 0, centre + 1.5, 315.0).unwrap();
    let mods = Modifiers { shift: true, ..Modifiers::COMMAND };
    // One drag of the first object, released with `release` held, or cancelled with Escape first.
    // The lines the gesture shows are the framework's (snapping and its axis lock are asserted
    // on where the copy lands).
    let drag = |h: &mut crate::table_resize::tests::H, release: Modifiers, cancel: bool, flicker: bool| {
        h.state_mut().run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 3)})).unwrap();
        h.event(Event::ModifiersChanged(mods));
        h.event(Event::PointerMoved(from));
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
        h.run_steps(1);
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.run_steps(1);
        }
        assert!(h.state().canvas.object_gesture().is_some(), "Ctrl+Shift starts a copy drag");
        if flicker {
            // Shift up and down again before the release.
            h.event(Event::ModifiersChanged(Modifiers::COMMAND));
            h.run_steps(1);
            h.event(Event::ModifiersChanged(mods));
            h.run_steps(1);
        }
        if cancel {
            // The framework reads Escape with Ctrl/Cmd held as a chord, not as Escape: the modifiers go up first
            // (the pointer stays down), as a person lets go of them to reach for the key.
            h.event(Event::ModifiersChanged(Modifiers::NONE));
            h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            h.run_steps(1);
            assert!(h.state().canvas.object_gesture().is_none(), "Escape ends the drag");
        }
        h.event(Event::ModifiersChanged(release));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: release });
        h.run_steps(3);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(1);
    };
    let copy_rect = |h: &mut crate::table_resize::tests::H| {
        let (pos, _) = selected(h.state()).expect("the copy is selected");
        h.state_mut().session.layout().object(&pos, 0).unwrap().rect
    };
    drag(&mut h, mods, false, false);
    let r = copy_rect(&mut h);
    assert!((r.x + 50.0 - centre).abs() < 0.01, "the free axis still snaps: {r:?}");
    assert!((r.y - 250.0).abs() < 0.01, "the nearby object's y guide must not move the copy: {r:?}");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.doc, before);
    // Shift up before the release frees the axis.
    drag(&mut h, Modifiers::COMMAND, false, false);
    let r = copy_rect(&mut h);
    assert!((r.y - 250.0).abs() > 10.0, "Shift up releases the axis: {r:?}");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.doc, before);
    // Shift up and down again restores the axis lock.
    drag(&mut h, mods, false, true);
    let r = copy_rect(&mut h);
    assert!((r.y - 250.0).abs() < 0.01, "Shift down restores the axis: {r:?}");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.doc, before);
    // Escape commits nothing.
    drag(&mut h, mods, true, false);
    assert_eq!(h.state().session.doc, before);
    assert_eq!(h.state().session.undo_depth(), depth);
}

/// Ctrl+Shift on another object still adds it to the selection, as do Shift and Ctrl.
#[test]
fn copy_axis_modifiers_keep_additive_object_clicks() {
    use egui::{Event, Modifiers, PointerButton};
    for mods in [Modifiers::SHIFT, Modifiers::COMMAND, Modifiers { shift: true, ..Modifiers::COMMAND }] {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), Default::default());
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        for x in [100, 350] {
            app.run("caret.docEnd", json!({})).unwrap();
            app.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 60})).unwrap();
            app.run("arrange.bounds", json!({"page": 0, "x": x, "y": 250})).unwrap();
        }
        app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 3)})).unwrap();
        let before = app.session.doc.clone();
        let depth = app.session.undo_depth();
        let mut h = rotation_window(app);
        let at = crate::canvas::page_to_screen(h.state_mut(), 0, 400.0, 280.0).unwrap();
        h.event(Event::ModifiersChanged(mods));
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: mods });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: mods });
        h.run_steps(2);
        assert!(h.state().canvas.object_gesture().is_none());
        assert_eq!(wordcraft_engine::cmd::objects::picked_objects(&h.state().session), vec![Pos::body(0, 0), Pos::body(0, 3)]);
        assert_eq!(h.state().session.doc, before);
        assert_eq!(h.state().session.undo_depth(), depth);
        if mods.command && mods.shift {
            h.run_steps(4);
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: mods });
            h.run_steps(1);
            h.event(Event::PointerMoved(at + egui::vec2(100.0, 20.0)));
            h.run_steps(1);
            assert!(h.state().canvas.object_gesture().is_some(), "a secondary selected object starts an aligned copy");
            h.event(Event::PointerButton { pos: at + egui::vec2(100.0, 20.0), button: PointerButton::Primary, pressed: false, modifiers: mods });
            h.run_steps(2);
            assert_eq!(h.state().session.doc.para_at(&Pos::body(0, 0)).unwrap().objects.len(), 3);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(h.state().session.doc, before);
        }
    }
}

/// A wheel scroll during an aligned copy must not move its locked document row
/// onto another page when the original page scrolls off screen. The wheel scrolls on every
/// platform; a middle press only autoscrolls off macOS, iOS and the web.
#[test]
fn ctrl_shift_drag_keeps_its_document_axis_after_a_scroll() {
    use egui::{Event, Modifiers, PointerButton};
    let text = vec!["Original long document for a scrolling copy."; 120].join("\n");
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::from_text(&text)), Default::default());
    app.run("view.commentsPane", json!({"value": false})).unwrap();
    app.run("caret.docStart", json!({})).unwrap();
    app.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 60})).unwrap();
    app.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
    app.ui.alignment_guides = false;
    let before = app.session.doc.clone();
    let mut h = rotation_window(app);
    assert!(h.state_mut().session.layout().pages.len() > 2);
    let from = crate::canvas::page_to_screen(h.state_mut(), 0, 150.0, 280.0).unwrap();
    let to = crate::canvas::page_to_screen(h.state_mut(), 0, 450.0, 280.0).unwrap();
    let mods = Modifiers { shift: true, ..Modifiers::COMMAND };
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
    h.run_steps(1);
    h.event(Event::PointerMoved(to));
    h.run_steps(1);
    let offset = h.state().canvas.view.offset.1;
    // A wheel scroll with the primary button held, in points so the distance does not depend on the platform's line size.
    // The canvas's wheel is the document scheme (the framework's): Ctrl/Cmd + wheel steps the zoom and Shift + wheel
    // scrolls sideways, so the wheel event itself carries no modifier (the held keys stay down through `mods`).
    for _ in 0..16 {
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -60.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        });
        h.run_steps(1);
    }
    assert!(h.state().canvas.view.offset.1 > offset + 500.0, "the wheel scrolls the canvas under the held button");
    assert!(h.state().canvas.object_gesture().is_some(), "the copy drag survives the scroll");
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
    h.run_steps(3);
    let (pos, _) = selected(h.state()).unwrap();
    let hit = h.state_mut().session.layout().object(&pos, 0).unwrap();
    assert_eq!(hit.page, 0);
    assert!((hit.rect.y - 250.0).abs() < 0.01);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.doc, before);
}

/// An inline Ctrl+Shift copy follows the text insertion cursor, without an axis constraint.
#[test]
fn ctrl_shift_drag_copies_an_inline_picture_at_the_text_cursor() {
    use egui::{Event, Modifiers, PointerButton};
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::from_text("First paragraph.\nSecond paragraph.")), Default::default());
    app.run("view.commentsPane", json!({"value": false})).unwrap();
    let image = InlineObject::Image {
        media: String::new(),
        w: 80.0,
        h: 40.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    };
    app.session.doc.insert_object(&Pos::body(1, 0), image, &Default::default()).unwrap();
    app.session.touch();
    app.run("select.range", json!({"anchor": Pos::body(1, 0), "focus": Pos::body(1, 3)})).unwrap();
    let before = app.session.doc.clone();
    let depth = app.session.undo_depth();
    let mut h = rotation_window(app);
    let o = h.state_mut().session.layout().object(&Pos::body(1, 0), 0).unwrap();
    let target = Pos::body(0, "First".len());
    let c = h.state_mut().session.layout().caret(&target).unwrap();
    let from = crate::canvas::page_to_screen(h.state_mut(), 0, o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0).unwrap();
    let to = crate::canvas::page_to_screen(h.state_mut(), 0, c.x + 0.5, c.top + c.height / 2.0).unwrap();
    let mods = Modifiers { shift: true, ..Modifiers::COMMAND };
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
    h.run_steps(1);
    for k in 1..=5 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 5.0)));
        h.run_steps(1);
    }
    assert!(h.state().canvas.object_gesture().is_some(), "a copy drag runs");
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
    h.run_steps(3);
    let (pos, image) = selected(h.state()).unwrap();
    assert_eq!(pos, target);
    assert!(!image.is_floating());
    assert_eq!(h.state().session.doc.para_at(&Pos::body(1, 0)).unwrap().objects.len(), 1, "the original remains inline");
    assert_eq!(h.state().session.undo_depth(), depth + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.doc, before);
}

/// An app whose first paragraph holds `obj`, and where it's laid out.
fn app_with(obj: InlineObject) -> (WordApp, ObjectHit) {
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    app.session.doc.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
    let hit = app.session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
    (app, hit)
}

#[test]
fn charts_get_no_handles_or_drag_but_shapes_do() {
    let chart = InlineObject::Graphic { w: 200.0, h: 100.0, alt: String::new(), float: Default::default(), graphic: Arc::new(Default::default()) };
    let (app, hit) = app_with(chart);
    assert!(fixed(&app, &hit));
    let shape = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 50.0,
        h: 50.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let (app, hit) = app_with(shape);
    assert!(!fixed(&app, &hit));
}

#[test]
fn editable_smart_art_takes_resize_handles_but_not_a_move() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    app.run("insert.smartArt", json!({"layout": "pyramid", "items": ["a", "b"]})).unwrap();
    let hit = app.session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
    assert!(!fixed(&app, &hit));
    assert!(size_only(&app, &hit));
    // Its body takes no move and its box has no rotate handle, while a handle still resizes.
    use crate::table_resize::tests::{H, button, steps};
    app.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h: H = rotation_window(app);
    let at = |h: &mut H, x: f32, y: f32| crate::canvas::page_to_screen(h.state_mut(), hit.page, x, y).unwrap();
    let drag = |h: &mut H, from: Pos2, by: egui::Vec2| {
        h.event(egui::Event::PointerMoved(from));
        button(h, from, true);
        h.step();
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(from + by * (k as f32 / 4.0)));
            h.step();
        }
        button(h, from + by, false);
        steps(h, 3);
    };
    let (cx, cy) = (hit.rect.x + hit.rect.w / 2.0, hit.rect.y + hit.rect.h / 2.0);
    let centre = at(&mut h, cx, cy);
    drag(&mut h, centre, egui::vec2(40.0, 30.0));
    let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap();
    assert!((r.rect.x - hit.rect.x).abs() < 0.01 && (r.rect.y - hit.rect.y).abs() < 0.01, "a body drag moves nothing: {:?}", r.rect);
    // Where the rotate handle of a box sits: nothing turns here, while the same drag turns a plain shape.
    assert_eq!(rotate_probe(&mut h), 0.0, "no rotate handle");
    let mut control: H = {
        let (mut app, hit) = app_with(floating_rect(100.0, 60.0));
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        app.run("select.range", json!({"anchor": hit.pos(), "focus": Pos { off: hit.off + OBJ.len_utf8(), ..hit.pos() }})).unwrap();
        rotation_window(app)
    };
    assert!(rotate_probe(&mut control).abs() > 1.0, "the same drag turns a shape that has the handle");
    // The right edge's handle resizes.
    let edge = at(&mut h, hit.rect.right(), cy);
    drag(&mut h, edge, egui::vec2(40.0, 0.0));
    let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap();
    assert!(r.rect.w > hit.rect.w + 1.0, "a handle resizes it: {:?} -> {:?}", hit.rect, r.rect);
    let app = h.state_mut();
    let (w0, h0) = (hit.rect.w, hit.rect.h);
    assert!(app.run("arrange.bounds", json!({"width": w0 / 2.0, "height": h0 / 2.0})).is_err());
    app.run("picture.size", json!({"width": w0 / 2.0, "height": h0 / 2.0, "lockAspect": false})).unwrap();
    let hit = app.session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
    assert!((hit.rect.w - w0 / 2.0).abs() < 1.0);
}
/// Drags from where a box's rotate handle sits (as the rotation tests place it) a quarter turn
/// round the object at the start of the body, and returns the angle it ends at.
fn rotate_probe(h: &mut crate::table_resize::tests::H) -> f32 {
    use crate::table_resize::tests::{button, steps};
    let o = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap();
    let (cx, cy) = (o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
    let center = crate::canvas::page_to_screen(h.state_mut(), o.page, cx, cy).unwrap();
    let top = crate::canvas::page_to_screen(h.state_mut(), o.page, cx, o.rect.y).unwrap();
    let radius = center.distance(top) + 22.0;
    let (start, end) = (center - egui::vec2(0.0, radius), center + egui::vec2(radius, 0.0));
    h.event(egui::Event::PointerMoved(start));
    button(h, start, true);
    h.step();
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 4.0)));
        h.step();
    }
    button(h, end, false);
    steps(h, 3);
    h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().spin.deg
}

/// Alignment guides: dragging a floating shape to within a few points of the page's centre
/// snaps its centre exactly onto it and shows the guide while dragging; the drop lands there,
/// and the guides go. Holding Alt drags freely.
#[test]
fn dragging_a_shape_snaps_to_the_page_centre_unless_alt() {
    use egui::{Event, Modifiers, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 50})).unwrap();
    a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let page = h.state_mut().session.layout().pages.first().map(|p| p.w - p.markup).unwrap();
    let centre = page / 2.0;
    let shape = |h: &mut egui_kittest::Harness<'_, WordApp>| h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    // Press on the shape's centre and drag it so its centre ends 1.5 pt right of the page's.
    let drag = |h: &mut egui_kittest::Harness<'_, WordApp>, alt: bool| {
        let r = shape(h);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let from = crate::canvas::page_to_screen(h.state_mut(), 0, cx, cy).unwrap();
        let to = crate::canvas::page_to_screen(h.state_mut(), 0, centre + 1.5, cy).unwrap();
        h.event(Event::ModifiersChanged(if alt { Modifiers::ALT } else { Modifiers::default() }));
        h.event(Event::PointerMoved(from));
        h.event(button(from, true));
        h.step();
        // Out past the drag threshold, then to the target.
        for at in [from + egui::vec2(0.0, 40.0), to + egui::vec2(0.0, 40.0), to] {
            h.event(Event::PointerMoved(at));
            h.step();
        }
        assert!(h.state().canvas.object_gesture().is_some(), "dragging");
        h.event(button(to, false));
        for _ in 0..3 {
            h.step();
        }
        h.event(Event::ModifiersChanged(Modifiers::default()));
        h.step();
    };
    // The lines shown are the framework's (`SnapSuspend::Alt` and the smart-guide painting); the
    // app's part is where the drop lands.
    drag(&mut h, false);
    assert!(h.state().canvas.object_gesture().is_none(), "dropped");
    let r = shape(&mut h);
    assert!((r.x + r.w / 2.0 - centre).abs() < 0.01, "snapped onto the page's centre: {r:?}");
    // Alt: no snapping; it lands where the pointer put it.
    drag(&mut h, true);
    let r = shape(&mut h);
    assert!((r.x + r.w / 2.0 - centre - 1.5).abs() < 0.01, "free with Alt: {r:?}");
}
/// #1738: a floating shape dragged from page 1 onto page 2 leaves the view where it is, even
/// though the command that drops it (`arrange.bounds`) is not a view command.
#[test]
fn dropping_a_shape_on_another_page_keeps_the_view_where_it_is() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 50})).unwrap();
    a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
    let end = Pos::body(0, wordcraft_doc::para::OBJ.len_utf8());
    a.run("caret.set", json!({"pos": end})).unwrap();
    a.run("text.insert", json!({"text": "one"})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    a.run("para.pageBreakBefore", json!({})).unwrap();
    a.run("text.insert", json!({"text": "two"})).unwrap();
    a.run("view.zoom", json!({"value": 30})).unwrap();
    a.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": end})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
    let from = crate::canvas::page_to_screen(h.state_mut(), 0, r.x + r.w / 2.0, r.y + r.h / 2.0).unwrap();
    let to = crate::canvas::page_to_screen(h.state_mut(), 1, 200.0, 300.0).unwrap();
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    h.event(Event::PointerMoved(from));
    h.event(button(from, true));
    h.step();
    for k in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
        h.step();
    }
    h.state_mut().canvas.scroll_to_caret = false;
    h.event(button(to, false));
    h.step();
    h.step();
    let landed = h.state_mut().session.layout().object_at(1, 200.0, 300.0, 0.0).is_some();
    assert!(landed, "the shape landed on page 2");
    assert!(!h.state().canvas.scroll_to_caret, "the drop scrolled to the caret");
}

/// A floating rectangle shape, `w` × `h`.
fn floating_rect(w: f32, h: f32) -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: wordcraft_doc::para::Float { wrap: Wrap::InFrontOfText, x: 10.0, y: 10.0, ..Default::default() },
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

/// The bars in `shape`, as (top end, bottom end, width): a line segment, a stroked path from its
/// first to its last point, and a filled rectangle from the middle of its top to the middle of its
/// bottom. The framework draws a caret as a path and the app draws the insertion cursor as a
/// rectangle, so a test of either looks at all three.
fn vertical_bars(shape: &egui::Shape) -> Vec<(egui::Pos2, egui::Pos2, f32)> {
    match shape {
        egui::Shape::LineSegment { points: [a, b], stroke } => vec![(*a, *b, stroke.width)],
        egui::Shape::Path(path) => match (path.points.first(), path.points.last()) {
            (Some(a), Some(b)) if path.points.len() >= 2 => vec![(*a, *b, path.stroke.width)],
            _ => Vec::new(),
        },
        egui::Shape::Rect(rect) => vec![(rect.rect.center_top(), rect.rect.center_bottom(), rect.rect.width())],
        egui::Shape::Vec(shapes) => shapes.iter().flat_map(vertical_bars).collect(),
        _ => Vec::new(),
    }
}

/// A floating line `w` wide and `h` tall (0: level, as Word files often store a rule).
fn floating_line(w: f32, h: f32, flip_v: bool) -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::Line,
        w,
        h,
        fill: None,
        stroke: Some(wordcraft_doc::Rgb(0, 0, 0)),
        stroke_width: 0.75,
        float: wordcraft_doc::para::Float { wrap: Wrap::InFrontOfText, x: 60.0, y: 120.0, flip_v, ..Default::default() },
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

/// A selected level line, pressed in its middle and dragged, moves and stays level: its
/// middle is its body, not a resize handle (a level line's top and bottom edge handles sit on
/// it there, and grabbing one gave it a height, so it came out slanted).
#[test]
fn a_level_line_dragged_by_its_middle_moves_and_stays_level() {
    use crate::table_resize::tests::{H, button, steps};
    for flip_v in [false, true] {
        let (mut app, hit) = app_with(floating_line(300.0, 0.0, flip_v));
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        app.run("select.range", json!({"anchor": hit.pos(), "focus": Pos { off: hit.off + OBJ.len_utf8(), ..hit.pos() }})).unwrap();
        let mut h: H = rotation_window(app);
        let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
        let at = |h: &mut H, x: f32, y: f32| crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap();
        let start = at(&mut h, r.x + r.w / 2.0, r.y);
        // Along the page and a little down, as a hand drags.
        let end = start + egui::vec2(60.0, 3.0);
        h.event(egui::Event::PointerMoved(start));
        button(&h, start, true);
        h.step();
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 4.0)));
            h.step();
        }
        assert!(h.state().canvas.object_gesture().is_some(), "flip_v {flip_v}: the middle moves it");
        // While it moves, it shows where the pointer has it (the page leaves it out
        // meanwhile), not only as its end handles back where it was: something is drawn along
        // the line's new place, between its ends.
        h.step();
        let img = h.render().unwrap();
        let ppp = h.ctx.pixels_per_point();
        let (x0, x1) = (((end.x - 100.0) * ppp) as u32, ((end.x + 100.0) * ppp) as u32);
        let (y0, y1) = (((end.y - 3.0) * ppp) as u32, ((end.y + 3.0) * ppp) as u32);
        let pointer = Rect::from_min_size(end - egui::vec2(2.0, 2.0), egui::vec2(20.0, 26.0));
        let drawn = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| !pointer.contains(egui::pos2(x as f32 / ppp, y as f32 / ppp)))
            .filter(|&(x, y)| img.get_pixel(x, y).0[..3].iter().any(|c| *c < 200))
            .count();
        assert!(drawn > 50, "flip_v {flip_v}: the dragged line shows at its new place ({drawn} pixels)");
        button(&h, end, false);
        steps(&mut h, 3);
        let n = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
        assert!(n.h.abs() < 0.01 && (n.w - r.w).abs() < 0.01, "flip_v {flip_v}: still a level line as long: {r:?} -> {n:?}");
        assert!(n.x > r.x + 10.0, "flip_v {flip_v}: moved along: {r:?} -> {n:?}");
        // Its end is a handle: dragged down, that end goes there and the other stays.
        let right = at(&mut h, n.right(), n.y);
        let down = right + egui::vec2(0.0, 40.0);
        h.event(egui::Event::PointerMoved(right));
        button(&h, right, true);
        h.step();
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(right + (down - right) * (k as f32 / 4.0)));
            h.step();
        }
        button(&h, down, false);
        steps(&mut h, 3);
        let e = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
        // Screen points per page point.
        let scale = (at(&mut h, n.x + 100.0, n.y).x - at(&mut h, n.x, n.y).x) / 100.0;
        assert!((e.x - n.x).abs() < 0.5 && (e.y - n.y).abs() < 0.5, "flip_v {flip_v}: the left end stays: {n:?} -> {e:?}");
        assert!(
            (e.right() - n.right()).abs() < 0.5 && (e.bottom() - (n.y + 40.0 / scale)).abs() < 0.5,
            "flip_v {flip_v}: the right end follows: {e:?}"
        );
    }
}

/// End handles go only to the lines and connectors `shape.connect` moves: floating ones in the
/// body text. One In Line with Text keeps its frame's handles, so it can still be resized on the
/// page.
#[test]
fn only_floating_lines_and_connectors_take_end_handles() {
    for kind in [ShapeKind::Line, ShapeKind::StraightConnector] {
        for (wrap, ends) in [(Wrap::InFrontOfText, true), (Wrap::Inline, false)] {
            let mut obj = floating_line(200.0, 40.0, false);
            if let InlineObject::Shape { kind: k, float, .. } = &mut obj {
                (*k, float.wrap) = (kind, wrap);
            }
            let (mut app, hit) = app_with(obj);
            app.run("select.range", json!({"anchor": hit.pos(), "focus": Pos { off: hit.off + OBJ.len_utf8(), ..hit.pos() }})).unwrap();
            let layout = app.session.layout();
            assert_eq!(connector_ends(&app, &layout).is_some(), ends, "{kind:?} {wrap:?}");
        }
    }
}

/// A text box doesn't snap to the objects inside it (they move with it).
#[test]
fn a_text_box_does_not_snap_to_its_own_contents() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    app.run("insert.textBox", json!({"text": "Box"})).unwrap();
    let bx = app.session.layout().find_object(0, |o| o.text_box.is_some()).expect("the box");
    let end = Pos { off: bx.off + OBJ.len_utf8(), ..bx.pos() };
    app.run("select.range", json!({"anchor": bx.pos(), "focus": end})).unwrap();
    app.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 200, "width": 200, "height": 150})).unwrap();
    let id = app.session.layout().find_object(0, |o| o.text_box.is_some()).and_then(|o| o.text_box).unwrap();
    let inner = Pos { story: StoryRef::Part(id), path: wordcraft_doc::Path::top(0), off: 0 };
    app.session.doc.insert_object(&inner, floating_rect(30.0, 20.0), &Default::default()).unwrap();
    app.session.touch();
    let layout = app.session.layout();
    let bx = layout.find_object(0, |o| o.text_box.is_some()).unwrap();
    let pg = layout.pages.first().unwrap();
    let inside = layout.find_object(0, |o| o.story == StoryRef::Part(id)).expect("the box's own object is laid out");
    // PLAN: assumed `snap_targets(pg: &Page, me: Option<&ObjectHit>) -> Vec<SnapTarget>`, with `Axis`, `SnapKind` and `SnapTarget` in scope through `objects.rs` (canvas-w11, S-L237)
    let targets = snap_targets(pg, Some(&bx));
    assert!(!has_target(&targets, Axis::Y, SnapKind::LayerEdge, inside.rect.x), "{targets:?}");
    assert!(!has_target(&targets, Axis::X, SnapKind::LayerEdge, inside.rect.y), "{targets:?}");
    // Another floating object on the page is still a target.
    let other = snap_targets(pg, Some(&inside));
    assert!(has_target(&other, Axis::Y, SnapKind::LayerEdge, bx.rect.x), "{other:?}");
    assert!(has_target(&other, Axis::X, SnapKind::LayerEdge, bx.rect.y), "{other:?}");
}

/// Whether `targets` holds a line of `kind` along `along` at page point `at`.
fn has_target(targets: &[SnapTarget], along: Axis, kind: SnapKind, at: f32) -> bool {
    targets.iter().any(|t| t.along == along && t.kind == kind && (t.position - f64::from(at)).abs() < 1e-3)
}

/// Pressing Alt in the middle of a drag stops the snapping, and letting it go again doesn't
/// turn keytips on (the Alt was part of the drag, not a tap).
#[test]
fn alt_pressed_during_a_drag_unsnaps_and_keeps_keytips_off() {
    use egui::{Event, Modifiers, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 50})).unwrap();
    a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let centre = h.state_mut().session.layout().pages.first().map(|p| (p.w - p.markup) / 2.0).unwrap();
    let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
    let cy = r.y + r.h / 2.0;
    let from = crate::canvas::page_to_screen(h.state_mut(), 0, r.x + r.w / 2.0, cy).unwrap();
    let to = crate::canvas::page_to_screen(h.state_mut(), 0, centre + 1.5, cy).unwrap();
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    h.event(Event::PointerMoved(from));
    h.event(button(from, true));
    h.step();
    for at in [from + egui::vec2(0.0, 40.0), to] {
        h.event(Event::PointerMoved(at));
        h.step();
    }
    assert!(h.state().canvas.object_gesture().is_some(), "dragging");
    // Alt down mid-drag, as a key and a modifier change (backends send both).
    let alt_key = |pressed: bool, modifiers: Modifiers| Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed, repeat: false, modifiers };
    h.event(alt_key(true, Modifiers::ALT));
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::PointerMoved(to + egui::vec2(0.0, 1.0)));
    h.step();
    assert!(h.state().canvas.object_gesture().is_some(), "still dragging");
    // Alt up, then the release.
    h.event(alt_key(false, Modifiers::default()));
    h.event(Event::ModifiersChanged(Modifiers::default()));
    h.event(button(to, false));
    for _ in 0..3 {
        h.step();
    }
    assert!(h.state().canvas.object_gesture().is_none());
    let landed = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
    assert!((landed.x + landed.w / 2.0 - centre - 1.5).abs() < 0.01, "free with Alt: {landed:?}");
    assert!(!h.state().keytips_active, "Alt during a drag isn't a keytip tap");
}

/// #2233, #2234: dragging an In Line with Text picture into another paragraph shows a bold
/// insertion cursor at the text under the pointer (no alignment guides), and the drop puts the
/// picture there, still inline and selected. Dropped back where it was, nothing changes.
#[test]
fn an_inline_picture_drags_to_the_insertion_cursor_and_stays_inline() {
    use crate::table_resize::tests::{H, button, steps};
    let mut a = WordApp::new(
        wordcraft_engine::Session::new(Document::from_text("The first paragraph of text.\nThe second paragraph, where the picture starts.")),
        Default::default(),
    );
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let picture = InlineObject::Image {
        media: String::new(),
        w: 80.0,
        h: 40.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    };
    a.session.doc.insert_object(&Pos::body(1, 0), picture, &Default::default()).unwrap();
    a.session.touch();
    let mut h: H = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    let o = h.state_mut().session.layout().object(&Pos::body(1, 0), 0).expect("laid out");
    assert_eq!(o.wrap, Wrap::Inline);
    // Aim between "first" and " paragraph" in the first paragraph.
    let target = Pos::body(0, "The first".len());
    let c = h.state_mut().session.layout().caret(&target).expect("caret");
    let at = |h: &mut H, x: f32, y: f32| crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap();
    let start = at(&mut h, o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
    let end = at(&mut h, c.x + 0.5, c.top + c.height / 2.0);
    // The text caret's stroke at the target, over a few frames so a blink off does not hide it.
    h.state_mut().run("select.range", json!({"anchor": target, "focus": target})).unwrap();
    let (caret_top, caret_bottom) = (at(&mut h, c.x, c.top), at(&mut h, c.x, c.top + c.height));
    let mut caret_width: Option<f32> = None;
    // The framework draws the caret as a stroked path and focus decides whether it shows at all.
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    // The blink runs on the wall clock and a scripted move does not restart it: restart it, so the caret is on
    // for its first half second however the clock stands when the test runs.
    h.state_mut().canvas.glide.restart(craft::platform::clock::monotonic_ms());
    // The framework gives the keyboard to no canvas by itself: step as the app's driver does until the page has the
    // keyboard it asked for (the request is the runtime's, taken at a frame's start), and only then look for the caret.
    for _ in 0..12 {
        crate::test_harness::frame(&mut h);
        if h.state().canvas.focused(&ctx) {
            break;
        }
        h.state_mut().canvas.focus_page(&ctx);
    }
    assert!(h.state().canvas.focused(&ctx), "the page takes the keyboard it asked for");
    // The caret fades in and may glide to the target: give it frames, and keep the blink in its on half.
    for _ in 0..30 {
        h.state_mut().canvas.glide.restart(craft::platform::clock::monotonic_ms());
        crate::test_harness::frame(&mut h);
        for s in &h.output().shapes {
            for (a, b, width) in vertical_bars(&s.shape) {
                if a.distance(caret_top) < 1.5 && b.distance(caret_bottom) < 1.5 {
                    caret_width = Some(caret_width.map_or(width, |w: f32| w.max(width)));
                }
            }
        }
    }
    let caret_width = caret_width.expect("the text caret is painted at the target");
    h.event(egui::Event::PointerMoved(start));
    h.step();
    button(&h, start, true);
    h.step();
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
        h.step();
    }
    assert!(h.state().canvas.object_gesture().is_some(), "dragging");
    // #4195: solid and twice the caret's width, so it can be seen, as in Word.
    let (top, bottom) = (at(&mut h, c.x, c.top), at(&mut h, c.x, c.top + c.height));
    let bold = h
        .output()
        .shapes
        .iter()
        .flat_map(|s| vertical_bars(&s.shape))
        .any(|(a, b, width)| a.distance(top) < 0.5 && b.distance(bottom) < 0.5 && width > caret_width);
    assert!(bold, "a bold insertion cursor from the line's top to its bottom, wider than the text caret");
    button(&h, end, false);
    steps(&mut h, 3);
    assert!(h.state().canvas.object_gesture().is_none());
    let (pos, obj) = selected(h.state()).expect("still selected");
    assert_eq!(pos, target, "landed at the insertion cursor");
    assert!(!obj.is_floating(), "still In Line with Text");
    assert_eq!(h.state().session.doc.para_at(&Pos::body(1, 0)).map(|p| p.objects.len()), Some(0), "left its old place");
    // Back onto itself: no change, no undo step.
    let undo = h.state().session.undo_depth();
    let o = h.state_mut().session.layout().object(&target, 0).expect("laid out");
    let start = at(&mut h, o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
    button(&h, start, true);
    h.step();
    for v in [egui::vec2(12.0, 0.0), egui::vec2(0.0, 0.0)] {
        h.event(egui::Event::PointerMoved(start + v));
        h.step();
    }
    button(&h, start, false);
    steps(&mut h, 3);
    assert_eq!(selected(h.state()).map(|(p, _)| p), Some(target));
    assert_eq!(h.state().session.undo_depth(), undo);
}

/// #1010: dragging a shape off text, the preview is the shape alone, not the text that was
/// behind it. An unfilled red rectangle in front of black text is dragged down into the empty
/// page: its preview has its red outline and nothing dark inside.
#[test]
fn a_dragged_shape_leaves_the_text_behind_it() {
    use crate::table_resize::tests::{H, button, gpu_turn, steps};
    let _gpu = gpu_turn();
    let text = "MMMMMMMMMMMM ".repeat(12);
    let mut a = WordApp::new(wordcraft_engine::Session::new(Document::from_text(&text)), Default::default());
    let shape = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 120.0,
        h: 60.0,
        fill: None,
        stroke: Some(wordcraft_doc::Rgb(0xFF, 0, 0)),
        stroke_width: 3.0,
        float: wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::InFrontOfText, ..Default::default() },
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    a.session.doc.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let mut h: H = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    let o = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
    let at = |h: &mut H, x: f32, y: f32| crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap();
    let down = 200.0;
    let start = at(&mut h, o.rect.x + o.rect.w / 2.0, o.rect.y + 4.0);
    let end = at(&mut h, o.rect.x + o.rect.w / 2.0, o.rect.y + 4.0 + down);
    h.event(egui::Event::PointerMoved(start));
    h.step();
    button(&h, start, true);
    h.step();
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
        h.step();
    }
    h.step(); // the preview is painted a frame after the move
    assert!(h.state().canvas.object_gesture().is_some(), "a drag is in progress");
    let (tl, br) = (at(&mut h, o.rect.x, o.rect.y + down), at(&mut h, o.rect.right(), o.rect.bottom() + down));
    let img = h.render().unwrap();
    let ppp = h.ctx.pixels_per_point();
    // Pixels in a screen area, but for the mouse pointer drawn at `end`.
    let count = |x0: f32, y0: f32, x1: f32, y1: f32, f: &dyn Fn([u8; 4]) -> bool| {
        let (x0, y0, x1, y1) = ((x0 * ppp).ceil() as u32, (y0 * ppp).ceil() as u32, (x1 * ppp) as u32, (y1 * ppp) as u32);
        let pointer = Rect::from_min_size(end - egui::vec2(2.0, 2.0), egui::vec2(20.0, 26.0));
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| !pointer.contains(egui::pos2(x as f32 / ppp, y as f32 / ppp)) && f(img.get_pixel(x, y).0))
            .count()
    };
    let red = count(tl.x - 4.0, tl.y - 4.0, br.x + 4.0, br.y + 4.0, &|[r, g, b, _]| r > 200 && g < 180 && b < 180);
    assert!(red > 50, "the preview shows the shape's outline ({red} red pixels)");
    let dark = count(tl.x + 8.0, tl.y + 8.0, br.x - 8.0, br.y - 8.0, &|[r, g, b, _]| r < 180 && g < 180 && b < 180);
    assert_eq!(dark, 0, "no text from behind the shape in its preview");
    button(&h, end, false);
    steps(&mut h, 3);
}

/// A 100×60 pt rectangle filled E2EFDA: [`fill_pixels`] counts the page's pixels of that colour.
fn filled_shape() -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 100.0,
        h: 60.0,
        fill: Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(0xE2, 0xEF, 0xDA))),
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

/// How many pixels of the first page have the fill colour: how many copies of the shape the
/// page shows.
fn fill_pixels(app: &mut WordApp, opts: &wordcraft_render::RenderOptions) -> usize {
    let layout = app.session.layout();
    let img = wordcraft_render::render_page(&app.session.doc, &layout.pages[0], 1.0, opts);
    img.to_straight().as_chunks::<4>().0.iter().filter(|p| (p[0], p[1], p[2]) == (0xE2, 0xEF, 0xDA)).count()
}

/// A shape dragged on the page is left out of it: the preview under the pointer stands in for
/// it, so the page must not draw it where it was as well (`DisplayOptions::hide_object`).
///
/// Judged by pixels: the page raster has none of its fill colour once it is left out, and a
/// thousand or so while it isn't. Without this, "it stays where it was while it is dragged"
/// would come back with nothing to catch it.
#[test]
fn a_dragged_inline_object_is_left_out_of_its_page() {
    let (mut app, hit) = app_with(filled_shape());
    let pos = hit.pos();
    let on = wordcraft_render::RenderOptions::default();
    let before = fill_pixels(&mut app, &on);
    assert!(before > 1000, "the shape is on its page to begin with: {before} px");
    let mut hidden = on.clone();
    hidden.display.hide_object = Some(pos);
    assert_eq!(fill_pixels(&mut app, &hidden), 0, "a dragged object is left out of its page");
}

/// The same for an object that is already floating: it is drawn as a `Placed::Shape` of its
/// own rather than inside a line, so leaving it out goes by the object the shape carries.
#[test]
fn a_dragged_floating_shape_is_left_out_of_its_page() {
    let (mut app, hit) = app_with(filled_shape());
    let pos = hit.pos();
    let end = Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() };
    let _ = app.run("select.range", serde_json::json!({"anchor": pos, "focus": end}));
    app.run("arrange.bounds", serde_json::json!({"page": 0, "x": 100.0, "y": 150.0})).expect("floats it");
    let hit = app.session.layout().object(&pos, 0).expect("laid out");
    assert!(hit.floating(), "arrange.bounds makes it float");

    let on = wordcraft_render::RenderOptions::default();
    let before = fill_pixels(&mut app, &on);
    assert!(before > 1000, "the floating shape is on its page to begin with: {before} px");
    let mut hidden = on.clone();
    hidden.display.hide_object = Some(pos);
    assert_eq!(fill_pixels(&mut app, &hidden), 0, "a dragged floating shape is left out of its page");
}

/// A floating shape dragged onto the next page lands there — the page the preview shows —
/// even dropped at the very top of it, where the anchor is the page's first paragraph. The
/// shape's wrap holds that paragraph on the old page, so once the shape moves the paragraph
/// can slide back and take the shape with it; the drop must re-anchor until it stays.
#[test]
fn a_shape_dragged_onto_the_next_page_is_dropped_there() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    for i in 0..40 {
        if i > 0 {
            a.run("text.newParagraph", json!({})).unwrap();
        }
        a.run("text.insert", json!({"text": format!("Paragraph {i} has words enough to run over two or three lines of the page.")})).unwrap();
    }
    a.run("caret.docStart", json!({})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250, "width": 100, "height": 50})).unwrap();
    assert!(a.session.layout().pages.len() > 1, "a second page to drag onto");
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 1800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let shape =
        |h: &mut egui_kittest::Harness<'_, WordApp>| h.state_mut().session.layout().find_object(0, |o| o.text_box.is_none()).expect("a shape");
    let r = shape(&mut h).rect;
    let from = crate::canvas::page_to_screen(h.state_mut(), 0, r.x + r.w / 2.0, r.y + r.h / 2.0).expect("the shape's page is on screen");
    let to = crate::canvas::page_to_screen(h.state_mut(), 1, 150.0, 120.0).expect("the next page is on screen");
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    h.event(Event::PointerMoved(from));
    h.event(button(from, true));
    h.step();
    for at in [from + egui::vec2(0.0, 40.0), to] {
        h.event(Event::PointerMoved(at));
        h.step();
    }
    assert!(h.state().canvas.object_gesture().is_some(), "dragging");

    h.event(button(to, false));
    for _ in 0..3 {
        h.step();
    }
    let o = shape(&mut h);
    assert_eq!(o.page, 1, "dropped on the page under the pointer, not thrown back");
    assert!((o.rect.x - 100.0).abs() < 0.5, "kept its place across: {:?}", o.rect);
}

/// A settled canvas for the rotation regressions, without taking focus on the desktop.
fn rotation_window(app: WordApp) -> crate::table_resize::tests::H {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    crate::table_resize::tests::steps(&mut h, 6);
    h
}

/// A dragged object's snap lines include the page margins and a neighbouring object's edges:
/// the app's target rule, which nothing else guards. It is read where it lives, on the targets
/// the page gives for the dragged object, not through a `Snap` outside a canvas pass.
#[test]
fn a_dragged_objects_snap_lines_include_the_page_margins_and_a_neighbouring_object() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), Default::default());
    app.run("view.commentsPane", json!({"value": false})).unwrap();
    app.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 60})).unwrap();
    app.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
    app.run("caret.docEnd", json!({})).unwrap();
    app.run("insert.shape", json!({"kind": "rectangle", "width": 80, "height": 40})).unwrap();
    app.run("arrange.bounds", json!({"page": 0, "x": 330, "y": 420})).unwrap();
    let layout = app.session.layout();
    let pg = &layout.pages[0];
    let body = pg.body;
    let me = layout.object(&Pos::body(0, 0), 0).expect("the dragged object is laid out");
    let neighbour = layout.object(&Pos::body(0, 3), 0).expect("the neighbour is laid out").rect;
    // PLAN: assumed `snap_targets(pg: &Page, me: Option<&ObjectHit>) -> Vec<SnapTarget>` (canvas-w11, S-L237)
    let targets = snap_targets(pg, Some(&me));
    assert!(has_target(&targets, Axis::Y, SnapKind::Guide, body.x), "the left margin is a target: {targets:?}");
    assert!(has_target(&targets, Axis::Y, SnapKind::Guide, body.right()), "the right margin is a target: {targets:?}");
    assert!(has_target(&targets, Axis::Y, SnapKind::LayerEdge, neighbour.x), "the neighbour's left edge is a target: {targets:?}");
    assert!(has_target(&targets, Axis::Y, SnapKind::LayerEdge, neighbour.right()), "the neighbour's right edge is a target: {targets:?}");
    assert!(!has_target(&targets, Axis::Y, SnapKind::LayerEdge, me.rect.x), "the dragged object is no target of its own drag: {targets:?}");
}

/// #2277: the actual picture pixels follow the rotation handle before release, including
/// pictures whose cached preview already contains a rotation and either reflection.
#[test]
fn rotation_drag_turns_picture_pixels_live_and_commits_once_or_cancels() {
    use crate::table_resize::tests::{H, button, gpu_turn, steps};
    let _gpu = gpu_turn();
    for (initial, flip_h, flip_v) in [(0.0, false, false), (30.0, true, false), (90.0, false, true), (315.0, true, true)] {
        // Four original colour quadrants make both accidental extra rotation and reflected
        // UVs observable. Sample near the long ends after a quarter turn, outside the old
        // picture, so the stationary page image cannot satisfy the pixel assertions.
        let colours = [[230, 20, 20, 255], [20, 200, 20, 255], [20, 20, 230, 255], [230, 200, 20, 255]];
        let image = image::RgbaImage::from_fn(80, 40, |x, y| {
            let quadrant = match (x < 40, y < 20) {
                (true, true) => 0,
                (false, true) => 1,
                (false, false) => 2,
                (true, false) => 3,
            };
            image::Rgba(colours[quadrant])
        });
        let bytes = wordcraft_geom::picture::encode_format(&image::DynamicImage::from(image.clone()), image::ImageFormat::Png).unwrap();
        let picture = InlineObject::Image {
            media: "rotation-test".into(),
            w: 200.0,
            h: 100.0,
            alt: String::new(),
            float: wordcraft_doc::para::Float { wrap: Wrap::InFrontOfText, x: 150.0, y: 180.0, rot: initial, flip_h, flip_v, ..Default::default() },
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        };
        let (mut app, _) = app_with(picture);
        app.session.doc.media.insert("rotation-test".into(), Arc::new(bytes));
        app.session.touch();
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, OBJ.len_utf8())})).unwrap();
        let mut h: H = rotation_window(app);
        let depth = h.state().session.undo_depth();
        let original = serde_json::to_value(&h.state().session.doc).unwrap();
        let o = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap();
        let (cx, cy) = (o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
        let center = crate::canvas::page_to_screen(h.state_mut(), 0, cx, cy).unwrap();
        let top = crate::canvas::page_to_screen(h.state_mut(), 0, cx, o.rect.y).unwrap();
        let radius = center.distance(top) + 22.0;
        let handle = |degrees: f32| center + egui::emath::Rot2::from_angle(degrees.to_radians()) * egui::vec2(0.0, -radius);
        let target = wordcraft_geom::normalize_degrees(initial + 90.0);
        let start = handle(initial);
        let end = handle(target);
        h.event(egui::Event::PointerMoved(start));
        button(&h, start, true);
        h.step();
        h.event(egui::Event::PointerMoved(end));
        steps(&mut h, 2);
        // PLAN: the rotate handle's distance from the box (`radius`) is assumed to reach `RotateZone::Handle` (cv.K37, canvas-w4)
        // The turn handle is the frame's (the framework's transform box), not the page tool's move gesture.
        assert!(h.state().canvas.frame_gesture(), "rotating");
        assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original, "dragging must not edit the document");
        assert_eq!(h.state().session.undo_depth(), depth);
        let rendered = h.render().unwrap();
        if initial == 0.0
            && let Ok(path) = std::env::var("WORDCRAFT_ROTATION_SHOT")
        {
            rendered.save(path).unwrap();
        }
        let ppp = h.ctx.pixels_per_point();
        let spin = wordcraft_geom::Spin::new(target, flip_h, flip_v);
        for ((x, y), colour) in [(-75.0, -20.0), (75.0, -20.0), (75.0, 20.0), (-75.0, 20.0)].into_iter().zip(colours) {
            let (x, y) = spin.apply(cx, cy, cx + x, cy + y);
            let at = crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap();
            let pixel = rendered.get_pixel((at.x * ppp).round() as u32, (at.y * ppp).round() as u32).0;
            for (actual, expected) in pixel.into_iter().zip(colour).take(3) {
                if expected > 150 {
                    assert!(actual > 170, "live quadrant missing: {initial}/{flip_h}/{flip_v} {colour:?} -> {pixel:?}");
                } else {
                    assert!(actual < 150, "live quadrant missing: {initial}/{flip_h}/{flip_v} {colour:?} -> {pixel:?}");
                }
            }
        }
        // A second angle reuses the raster rather than regenerating it on pointer moves.
        let textures = |h: &H| {
            h.output().shapes.iter().filter_map(|s| if let egui::Shape::Mesh(m) = &s.shape { Some(m.texture_id) } else { None }).collect::<Vec<_>>()
        };
        let before_move = textures(&h);
        let next = handle(target + 15.0);
        h.event(egui::Event::PointerMoved(next));
        steps(&mut h, 2);
        assert_eq!(textures(&h), before_move, "{initial}/{flip_h}/{flip_v}: the preview keeps its texture across moves");
        let final_angle = wordcraft_geom::normalize_degrees(target + 15.0);
        button(&h, next, false);
        steps(&mut h, 3);
        assert!(h.state().canvas.object_gesture().is_none());
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one command on release");
        let turned = (selected(h.state()).unwrap().1.frame().unwrap().2.rot - final_angle).rem_euclid(360.0);
        assert!(turned.min(360.0 - turned) < 1.0, "released at the angle the handle was dragged to");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        steps(&mut h, 3);
        assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original, "one undo restores everything");

        h.event(egui::Event::PointerMoved(start));
        button(&h, start, true);
        h.step();
        h.event(egui::Event::PointerMoved(end));
        steps(&mut h, 2);
        assert!(h.state().canvas.frame_gesture(), "rotating again");
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.step();
        button(&h, end, false);
        steps(&mut h, 3);
        assert!(h.state().canvas.object_gesture().is_none());
        assert_eq!(h.state().session.undo_depth(), depth, "Escape adds no undo step");
        assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original, "Escape cancels without an edit");
    }
}

#[test]
fn rotation_drag_keeps_text_boxes_outline_only_until_their_text_can_rotate() {
    use crate::table_resize::tests::{button, steps};
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), Default::default());
    app.run("insert.textBox", json!({"text": "Text stays upright on release"})).unwrap();
    let bx = app.session.layout().find_object(0, |o| o.text_box.is_some()).unwrap();
    app.run("select.range", json!({"anchor": bx.pos(), "focus": Pos { off: bx.off + OBJ.len_utf8(), ..bx.pos() }})).unwrap();
    app.run("arrange.bounds", json!({"page": 0, "x": 150, "y": 180, "width": 200, "height": 100})).unwrap();
    let mut h = rotation_window(app);
    let original = serde_json::to_value(&h.state().session.doc).unwrap();
    let depth = h.state().session.undo_depth();
    let bx = h.state_mut().session.layout().find_object(0, |o| o.text_box.is_some()).unwrap();
    let (cx, cy) = (bx.rect.x + bx.rect.w / 2.0, bx.rect.y + bx.rect.h / 2.0);
    let center = crate::canvas::page_to_screen(h.state_mut(), 0, cx, cy).unwrap();
    let top = crate::canvas::page_to_screen(h.state_mut(), 0, cx, bx.rect.y).unwrap();
    let radius = center.distance(top) + 22.0;
    let (start, end) = (center - egui::vec2(0.0, radius), center + egui::vec2(radius, 0.0));
    // The frame's textured meshes at rest: a preview texture would add one.
    let meshes = |h: &crate::table_resize::tests::H| h.output().shapes.iter().filter(|s| matches!(s.shape, egui::Shape::Mesh(_))).count();
    let at_rest = meshes(&h);
    h.event(egui::Event::PointerMoved(start));
    button(&h, start, true);
    h.step();
    h.event(egui::Event::PointerMoved(end));
    steps(&mut h, 2);
    assert!(h.state().canvas.frame_gesture(), "turning the text box frame");
    assert_eq!(meshes(&h), at_rest, "do not preview rotated text that would snap upright on release");
    assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original);
    assert_eq!(h.state().session.undo_depth(), depth);
    button(&h, end, false);
    steps(&mut h, 3);
    // The box turns by f32 trigonometry about its pivot: a quarter turn lands within rounding of 90°.
    let turned = selected(h.state()).unwrap().1.frame().unwrap().2.rot;
    assert!((turned - 90.0).abs() < 0.01, "turned a quarter: {turned}");
    assert_eq!(h.state().session.undo_depth(), depth + 1);
}

/// A large picture's drag preview remains a legal device texture.
#[test]
fn dragging_a_large_picture_renders_a_preview_within_the_gpu_texture_limit() {
    use crate::table_resize::tests::{H, button, steps};
    use egui::Event;
    let image = image::RgbaImage::from_pixel(16, 16, image::Rgba([30, 120, 210, 255]));
    let bytes = wordcraft_geom::picture::encode_format(&image::DynamicImage::from(image.clone()), image::ImageFormat::Png).unwrap();
    let picture = InlineObject::Image {
        media: "large-preview-test".into(),
        w: 14_400.0,
        h: 200.0,
        alt: String::new(),
        float: wordcraft_doc::para::Float { wrap: Wrap::InFrontOfText, x: 100.0, y: 100.0, ..Default::default() },
        crop: [0.0; 4],
        ole: None,
        fx: Default::default(),
    };
    let (mut app, _) = app_with(picture);
    app.session.doc.media.insert("large-preview-test".into(), Arc::new(bytes));
    app.session.touch();
    app.run("view.commentsPane", json!({"value": false})).unwrap();
    app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, OBJ.len_utf8())})).unwrap();
    let mut h: H = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    steps(&mut h, 6);
    let hit = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
    let max_side = h.ctx.input(|i| i.max_texture_side);
    assert!(hit.bounds().w * h.ctx.pixels_per_point() > max_side as f32, "unclamped picture exceeds the texture limit");
    let at = crate::canvas::page_to_screen(h.state_mut(), hit.page, hit.rect.x + 10.0, hit.rect.y + 10.0).expect("the picture center is visible");
    let meshes = |h: &H| h.output().shapes.iter().filter(|s| matches!(s.shape, egui::Shape::Mesh(_))).count();
    let at_rest = meshes(&h);
    h.event(Event::PointerMoved(at));
    button(&h, at, true);
    h.step();
    let moved = at + egui::vec2(24.0, 0.0);
    h.event(Event::PointerMoved(moved));
    steps(&mut h, 2);
    assert!(h.state().canvas.object_gesture().is_some(), "picture drag started");
    assert!(meshes(&h) > at_rest, "the drag paints a preview texture beside the page's");
    // The preview is a textured mesh of the frame; its uploaded texture is what the device limit is about.
    let textures = h.ctx.tex_manager();
    let textures = textures.read();
    let mut sized = 0;
    for shape in &h.output().shapes {
        if let egui::Shape::Mesh(mesh) = &shape.shape
            && let Some(meta) = textures.meta(mesh.texture_id)
        {
            let [width, height] = meta.size;
            assert!(width <= max_side && height <= max_side, "texture is {width}×{height}, texture limit is {max_side}");
            sized += 1;
        }
    }
    assert!(sized > 0, "the frame paints textures, the large preview among them");
}

/// #6027: a click, or a double-click, in the text of a text box inside a group puts the caret
/// there, and typing edits that text; the group stays. Before, the click selected the whole
/// group and typing replaced it.
#[test]
fn typing_after_a_click_in_a_grouped_text_box_edits_its_text() {
    use egui::{Event, PointerButton};
    for clicks in [1, 2] {
        let mut d = Document::new();
        let id = d.add_part(
            wordcraft_doc::PartKind::TextBox,
            vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Card", Default::default()))],
        );
        let shape = |kind, h, story| InlineObject::Shape {
            kind,
            w: 200.0,
            h,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float: Default::default(),
            story,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        let float = wordcraft_doc::para::Float {
            wrap: wordcraft_doc::para::Wrap::InFrontOfText,
            h_rel: wordcraft_doc::para::Anchor::Page,
            v_rel: wordcraft_doc::para::Anchor::Page,
            x: 150.0,
            y: 200.0,
            ..Default::default()
        };
        let group = InlineObject::Group {
            w: 200.0,
            h: 100.0,
            float,
            ch_w: 200.0,
            ch_h: 100.0,
            children: vec![
                wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: shape(ShapeKind::Rectangle, 30.0, None) },
                wordcraft_doc::para::GroupChild { x: 0.0, y: 40.0, obj: shape(ShapeKind::TextBox, 60.0, Some(id)) },
            ],
            canvas: None,
        };
        d.insert_object(&Pos::body(0, 0), group, &Default::default()).unwrap();
        let mut a = WordApp::new(wordcraft_engine::Session::new(d), crate::Services::default());
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        // In the box's text, right of "Card" (the box starts 40 pt into the group).
        let at = crate::canvas::page_to_screen(h.state_mut(), 0, 150.0 + 120.0, 200.0 + 52.0).unwrap();
        h.event(Event::PointerMoved(at));
        for _ in 0..clicks {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        }
        for _ in 0..3 {
            h.step();
        }
        h.event(Event::Text("!".into()));
        for _ in 0..3 {
            h.step();
        }
        let doc = &h.state().session.doc;
        let text: String = doc.parts.get(&id).unwrap().blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect();
        assert!(text.contains('!'), "{clicks} click(s): typed into the box: {text:?}");
        assert!(
            matches!(doc.body.first().and_then(|b| b.as_para()).and_then(|p| p.objects.first()), Some(InlineObject::Group { .. })),
            "{clicks} click(s): the group is still there"
        );
    }
}

/// #2637: inside a Drawing Canvas the mouse works on the members, as in Word. A click on a
/// shape picks that shape (the canvas stays the selection in the text) and shows its frame;
/// dragging it moves it inside the canvas; a click in a text box member's text puts the
/// caret there. Before, the canvas took every click, as one static object.
#[test]
fn clicking_and_dragging_inside_a_canvas_works_on_its_members() {
    use egui::{Event, PointerButton};
    use wordcraft_engine::cmd::canvas::selected_member;
    let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.canvas", json!({"width": 400, "height": 300})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 40})).unwrap();
    a.run("canvas.member", json!({"index": 0, "x": 40, "y": 30})).unwrap();
    a.run("insert.textBox", json!({"text": "Label"})).unwrap();
    a.run("select.objects", json!({"index": 0})).unwrap();
    a.run("canvas.member", json!({"index": 1, "x": 200, "y": 100, "width": 150, "height": 80})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let frame = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).expect("the canvas is laid out").rect;
    let at = |h: &mut egui_kittest::Harness<'_, WordApp>, x: f32, y: f32| {
        crate::canvas::page_to_screen(h.state_mut(), 0, frame.x + x, frame.y + y).unwrap()
    };
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    let members =
        |h: &egui_kittest::Harness<'_, WordApp>| match h.state().session.doc.body.first().and_then(|b| b.as_para()).and_then(|p| p.objects.first()) {
            Some(InlineObject::Group { children, .. }) => children.iter().map(|c| (c.x, c.y)).collect::<Vec<_>>(),
            o => panic!("{o:?}"),
        };

    // A click on the rectangle picks it.
    let on_rect = at(&mut h, 90.0, 50.0);
    h.event(Event::PointerMoved(on_rect));
    h.event(button(on_rect, true));
    h.step();
    h.event(button(on_rect, false));
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(selected_member(&h.state().session).map(|(_, i, _)| i), Some(0), "the rectangle is picked, not the whole canvas");
    let shown = {
        let app = h.state_mut();
        let layout = app.session.layout();
        active(app, &layout).map(|(o, _)| (o.member, o.rect))
    };
    assert_eq!(shown, Some((Some(0), wordcraft_geom::Rect::new(frame.x + 40.0, frame.y + 30.0, 100.0, 40.0))), "its frame is the one shown");

    // Dragged 50 pt right and 20 pt down, it moves inside the canvas; the canvas stays put.
    let to = at(&mut h, 140.0, 70.0);
    h.event(Event::PointerMoved(on_rect));
    h.event(button(on_rect, true));
    h.step();
    for p in [on_rect + egui::vec2(0.0, 30.0), to + egui::vec2(0.0, 30.0), to] {
        h.event(Event::PointerMoved(p));
        h.step();
    }
    h.event(button(to, false));
    for _ in 0..3 {
        h.step();
    }
    let moved = members(&h);
    let near = |(x, y): (f32, f32), (ex, ey): (f32, f32)| (x - ex).abs() < 0.01 && (y - ey).abs() < 0.01;
    assert!(near(moved[0], (90.0, 50.0)) && near(moved[1], (200.0, 100.0)), "{moved:?}");
    assert_eq!(h.state_mut().session.layout().object(&Pos::body(0, 0), 0).map(|o| o.rect), Some(frame));

    // A click in the text box's text puts the caret in it.
    let in_box = at(&mut h, 275.0, 140.0);
    h.event(Event::PointerMoved(in_box));
    h.event(button(in_box, true));
    h.step();
    h.event(button(in_box, false));
    for _ in 0..3 {
        h.step();
    }
    let story = h.state().session.sel.focus.story;
    let is_box = matches!(story, StoryRef::Part(id) if h.state().session.doc.parts.get(&id).is_some_and(|p| p.kind == PartKind::TextBox));
    assert!(is_box, "the caret is in the text box's text: {story:?}");
}

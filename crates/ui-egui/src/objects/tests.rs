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
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 1100.0)).build_ui_state(
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
        let d = h.state().canvas.obj_drag.as_ref().expect("Ctrl+Shift starts a copy drag");
        assert!(d.copy);
        assert!((d.drag.rect.x - want_x).abs() < 0.01 && (d.drag.rect.y - want_y).abs() < 0.01, "aligned preview: {:?}", d.drag.rect);
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
        h.run_steps(3);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(1);
        assert!(h.state().canvas.obj_drag.is_none());
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
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
    h.run_steps(1);
    for k in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
        h.run_steps(1);
    }
    let d = h.state().canvas.obj_drag.as_ref().unwrap();
    assert!((d.drag.rect.x + 50.0 - centre).abs() < 0.01, "the free axis still snaps");
    assert!((d.drag.rect.y - 250.0).abs() < 0.01, "the nearby object's y guide must not move the copy");
    assert!(d.guides.iter().all(|g| g.axis == guides::Axis::X));
    assert!(!d.guides.is_empty());
    h.event(Event::ModifiersChanged(Modifiers::COMMAND));
    h.run_steps(1);
    assert!((h.state().canvas.obj_drag.as_ref().unwrap().drag.rect.y - 250.0).abs() > 10.0, "Shift up releases the axis");
    h.event(Event::ModifiersChanged(mods));
    h.run_steps(1);
    assert!((h.state().canvas.obj_drag.as_ref().unwrap().drag.rect.y - 250.0).abs() < 0.01, "Shift down restores the axis");
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: mods });
    h.run_steps(1);
    assert!(h.state().canvas.obj_drag.is_none());
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
    h.run_steps(2);
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
        assert!(h.state().canvas.obj_drag.is_none());
        assert_eq!(wordcraft_engine::cmd::objects::picked_objects(&h.state().session), vec![Pos::body(0, 0), Pos::body(0, 3)]);
        assert_eq!(h.state().session.doc, before);
        assert_eq!(h.state().session.undo_depth(), depth);
        if mods.command && mods.shift {
            h.run_steps(4);
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: mods });
            h.run_steps(1);
            h.event(Event::PointerMoved(at + egui::vec2(100.0, 20.0)));
            h.run_steps(1);
            assert!(h.state().canvas.obj_drag.as_ref().is_some_and(|d| d.copy), "a secondary selected object starts an aligned copy");
            h.event(Event::PointerButton { pos: at + egui::vec2(100.0, 20.0), button: PointerButton::Primary, pressed: false, modifiers: mods });
            h.run_steps(2);
            assert_eq!(h.state().session.doc.para_at(&Pos::body(0, 0)).unwrap().objects.len(), 3);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(h.state().session.doc, before);
        }
    }
}

/// A middle-button scroll during an aligned copy must not move its locked document row
/// onto another page when the original page scrolls off screen.
#[test]
fn ctrl_shift_drag_keeps_its_document_axis_after_middle_scroll() {
    use egui::{Event, Modifiers, PointerButton};
    let text = vec!["Original long document for a scrolling copy."; 120].join("\n");
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::from_text(&text)), Default::default());
    app.run("view.commentsPane", json!({"value": false})).unwrap();
    app.run("caret.docStart", json!({})).unwrap();
    app.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 60})).unwrap();
    app.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
    app.ui.alignment_guides = false;
    app.canvas.auto_scroll.supported = true;
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
    let offset = h.state().canvas.scroll_offset.y;
    h.event(Event::PointerButton { pos: to, button: PointerButton::Middle, pressed: true, modifiers: mods });
    h.run_steps(1);
    let below = to + egui::vec2(0.0, 150.0);
    h.event(Event::PointerMoved(below));
    h.run_steps(16);
    assert!(h.state().canvas.scroll_offset.y > offset + 500.0, "the real middle-button gesture scrolls the canvas");
    h.event(Event::PointerButton { pos: below, button: PointerButton::Middle, pressed: false, modifiers: mods });
    h.run_steps(3);
    let d = h.state().canvas.obj_drag.as_ref().unwrap();
    assert_eq!(d.drag.to_page, 0, "the copy stays on the original horizontal row");
    assert!((d.drag.rect.y - 250.0).abs() < 0.01, "the document y stays locked after scrolling");
    h.event(Event::PointerButton { pos: below, button: PointerButton::Primary, pressed: false, modifiers: mods });
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
    let image =
        InlineObject::Image { media: String::new(), w: 80.0, h: 40.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
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
    let d = h.state().canvas.obj_drag.as_ref().unwrap();
    assert!(d.copy);
    assert_eq!(d.drop_at.as_ref().map(|(p, _)| p), Some(&target));
    assert!(d.guides.is_empty());
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
    // Its body shows no move cursor; its handles show theirs.
    assert_eq!(hover_cursor(&app, &hit, Grab::Move), egui::CursorIcon::Default);
    assert_ne!(hover_cursor(&app, &hit, Grab::Resize(frame::Handle { x: 1, y: 0 })), egui::CursorIcon::Default);
    let (w0, h0) = (hit.rect.w, hit.rect.h);
    assert!(app.run("arrange.bounds", json!({"width": w0 / 2.0, "height": h0 / 2.0})).is_err());
    app.run("picture.size", json!({"width": w0 / 2.0, "height": h0 / 2.0, "lockAspect": false})).unwrap();
    let hit = app.session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
    assert!((hit.rect.w - w0 / 2.0).abs() < 1.0);
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
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
        let d = h.state().canvas.obj_drag.as_ref().expect("dragging");
        let mid = (d.drag.rect, d.guides.clone());
        h.event(button(to, false));
        for _ in 0..3 {
            h.step();
        }
        h.event(Event::ModifiersChanged(Modifiers::default()));
        h.step();
        mid
    };
    let (rect, guides) = drag(&mut h, false);
    assert!((rect.x + rect.w / 2.0 - centre).abs() < 1e-3, "snapped onto the page's centre while dragging: {rect:?}");
    assert!(guides.contains(&crate::guides::Guide::x(centre)), "the centre guide shows: {guides:?}");
    assert!(h.state().canvas.obj_drag.is_none(), "dropped: no guides any more");
    let r = shape(&mut h);
    assert!((r.x + r.w / 2.0 - centre).abs() < 0.01, "dropped on the centre: {r:?}");
    // Alt: no snapping, no guides; it lands where the pointer put it.
    let (rect, guides) = drag(&mut h, true);
    assert!((rect.x + rect.w / 2.0 - centre - 1.5).abs() < 0.01, "free with Alt: {rect:?}");
    assert!(guides.is_empty(), "{guides:?}");
    let r = shape(&mut h);
    assert!((r.x + r.w / 2.0 - centre - 1.5).abs() < 0.01, "{r:?}");
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
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
    let targets = snap_targets(pg, &bx);
    assert!(!targets.contains(&inside.bounds()), "{targets:?}");
    // Another floating object on the page is still a target.
    let other = snap_targets(pg, &inside);
    assert!(other.contains(&bx.bounds()), "{other:?}");
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
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
    assert!(!h.state().canvas.obj_drag.as_ref().expect("dragging").guides.is_empty(), "snapped before Alt");
    // Alt down mid-drag, as a key and a modifier change (backends send both).
    let alt_key = |pressed: bool, modifiers: Modifiers| Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed, repeat: false, modifiers };
    h.event(alt_key(true, Modifiers::ALT));
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::PointerMoved(to + egui::vec2(0.0, 1.0)));
    h.step();
    let d = h.state().canvas.obj_drag.as_ref().expect("still dragging");
    assert!(d.guides.is_empty() && (d.drag.rect.x + d.drag.rect.w / 2.0 - centre - 1.5).abs() < 0.01, "free with Alt");
    // Alt up, then the release.
    h.event(alt_key(false, Modifiers::default()));
    h.event(Event::ModifiersChanged(Modifiers::default()));
    h.event(button(to, false));
    for _ in 0..3 {
        h.step();
    }
    assert!(h.state().canvas.obj_drag.is_none());
    assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Off, "Alt during a drag isn't a keytip tap");
}

/// #2233, #2234: dragging an In Line with Text picture into another paragraph shows a dashed
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
    let picture =
        InlineObject::Image { media: String::new(), w: 80.0, h: 40.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
    a.session.doc.insert_object(&Pos::body(1, 0), picture, &Default::default()).unwrap();
    a.session.touch();
    let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
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
    h.event(egui::Event::PointerMoved(start));
    h.step();
    button(&h, start, true);
    h.step();
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
        h.step();
    }
    let d = h.state().canvas.obj_drag.as_ref().expect("dragging");
    assert_eq!(d.drop_at.as_ref().map(|(p, _)| p.clone()), Some(target.clone()), "the insertion cursor is at the text under the pointer");
    assert!(d.guides.is_empty(), "an inline object doesn't snap to guides");
    button(&h, end, false);
    steps(&mut h, 3);
    assert!(h.state().canvas.obj_drag.is_none());
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
    use crate::table_resize::tests::{H, button, steps};
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
    let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
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
    assert!(h.state().canvas.obj_drag.as_ref().is_some_and(|d| d.drag.moved), "a drag is in progress");
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
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 1800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
    assert_eq!(h.state().canvas.obj_drag.as_ref().expect("dragging").drag.to_page, 1, "the preview is on the page under the pointer");
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
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
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

/// #2277: the actual picture pixels follow the rotation handle before release, including
/// pictures whose cached preview already contains a rotation and either reflection.
#[test]
fn rotation_drag_turns_picture_pixels_live_and_commits_once_or_cancels() {
    use crate::table_resize::tests::{H, button, steps};
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
        let mut bytes = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let picture = InlineObject::Image {
            media: "rotation-test".into(),
            w: 200.0,
            h: 100.0,
            alt: String::new(),
            float: wordcraft_doc::para::Float { wrap: Wrap::InFrontOfText, x: 150.0, y: 180.0, rot: initial, flip_h, flip_v, ..Default::default() },
            crop: [0.0; 4],
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
        let d = h.state().canvas.obj_drag.as_ref().expect("rotating");
        assert_eq!(d.drag.grab, Grab::Rotate);
        assert_eq!(d.drag.deg, target);
        let texture = d.preview.as_ref().expect("cached picture preview").id();
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
        let next = handle(target + 15.0);
        h.event(egui::Event::PointerMoved(next));
        steps(&mut h, 2);
        assert_eq!(h.state().canvas.obj_drag.as_ref().unwrap().preview.as_ref().unwrap().id(), texture);
        let final_angle = h.state().canvas.obj_drag.as_ref().unwrap().drag.deg;
        button(&h, next, false);
        steps(&mut h, 3);
        assert!(h.state().canvas.obj_drag.is_none());
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one command on release");
        assert_eq!(selected(h.state()).unwrap().1.frame().unwrap().2.rot, final_angle);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        steps(&mut h, 3);
        assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original, "one undo restores everything");

        h.event(egui::Event::PointerMoved(start));
        button(&h, start, true);
        h.step();
        h.event(egui::Event::PointerMoved(end));
        steps(&mut h, 2);
        assert!(h.state().canvas.obj_drag.as_ref().unwrap().preview.is_some());
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.step();
        button(&h, end, false);
        steps(&mut h, 3);
        assert!(h.state().canvas.obj_drag.is_none());
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
    h.event(egui::Event::PointerMoved(start));
    button(&h, start, true);
    h.step();
    h.event(egui::Event::PointerMoved(end));
    steps(&mut h, 2);
    let d = h.state().canvas.obj_drag.as_ref().expect("turning the text box frame");
    assert_eq!((d.drag.grab, d.drag.deg), (Grab::Rotate, 90.0));
    assert!(d.preview.is_none(), "do not preview rotated text that would snap upright on release");
    assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original);
    assert_eq!(h.state().session.undo_depth(), depth);
    button(&h, end, false);
    steps(&mut h, 3);
    assert_eq!(selected(h.state()).unwrap().1.frame().unwrap().2.rot, 90.0);
    assert_eq!(h.state().session.undo_depth(), depth + 1);
}

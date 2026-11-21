use super::*;

fn window(ppp: f32) -> H {
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Drawing cursor test")), Default::default());
    let mut h = crate::test_harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(ppp).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    steps(&mut h, 6);
    h
}

fn hover_page(h: &mut H) -> Pos2 {
    steps(h, 2);
    let at = page_to_screen(h.state_mut(), 0, 140.0, 160.0).unwrap();
    h.event(egui::Event::PointerMoved(at));
    steps(h, 2);
    at
}

fn footprint(h: &H, at: Pos2, color: Color32) -> f32 {
    h.output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Circle(c) if c.center.distance(at) < 0.1 && c.fill == color => Some(c.radius),
            _ => None,
        })
        .expect("the cursor shows the ink footprint at the drawing hotspot")
}

#[test]
fn drawing_tools_have_distinct_visible_cursors_without_editing() {
    let mut h = window(1.0);
    let before = h.state().session.doc.clone();
    let mut appearances = Vec::new();
    for command in ["draw.pen", "draw.pencil", "draw.highlighter", "draw.eraser"] {
        h.state_mut().run(command, json!({})).unwrap();
        let at = hover_page(&mut h);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "{command} replaces the generic crosshair");
        let near = Rect::from_center_size(at, vec2(180.0, 180.0));
        let shapes: Vec<_> =
            h.output().shapes.iter().filter(|s| near.contains_rect(s.shape.visual_bounding_rect())).map(|s| format!("{:?}", s.shape)).collect();
        assert!(!shapes.is_empty(), "the hidden system pointer has a visible replacement");
        assert!(!appearances.contains(&shapes), "{command} is visually distinct from the other tools");
        appearances.push(shapes);
        assert_eq!(h.state().session.doc, before);
        assert!(!h.state().session.dirty);
    }
}

#[test]
fn drawing_cursors_stay_compact_without_floating_tool_badges() {
    let mut h = window(1.0);
    for command in ["draw.pen", "draw.pencil", "draw.highlighter", "draw.eraser"] {
        h.state_mut().run(command, json!({"color": "CC2244", "width": 6.0})).unwrap();
        let at = hover_page(&mut h);
        let near = Rect::from_center_size(at, vec2(120.0, 120.0));
        let mut bounds = Rect::NOTHING;
        let mut frames = 0;
        for shape in &h.output().shapes {
            let rect = shape.shape.visual_bounding_rect();
            if near.contains_rect(rect) && rect.is_positive() {
                bounds = bounds.union(rect);
                frames += usize::from(matches!(shape.shape, egui::Shape::Rect(_)));
            }
        }
        assert!(bounds.contains(at), "{command} marks the real drawing hotspot");
        assert!(Rect::from_center_size(at, vec2(48.0, 48.0)).contains_rect(bounds), "{command} stays close to its hotspot: {bounds:?}");
        if command == "draw.highlighter" {
            assert_eq!(frames, 2, "a light and dark square nib outline distinguishes the highlighter");
        } else {
            assert_eq!(frames, 0, "{command} has no floating button frame");
        }
    }
}

#[test]
fn cursor_footprint_tracks_colour_width_zoom_and_pixel_density() {
    for ppp in [1.0, 2.0] {
        let mut h = window(ppp);
        for (command, alpha) in [("draw.pen", 255), ("draw.pencil", 217), ("draw.highlighter", 128)] {
            for (width, color, rgb) in [(2.0, "CC2244", (204, 34, 68)), (8.0, "2266CC", (34, 102, 204))] {
                h.state_mut().run(command, json!({"color": color, "width": width})).unwrap();
                for zoom in [50, 100] {
                    h.state_mut().run("view.zoom", json!({"value": zoom})).unwrap();
                    let at = hover_page(&mut h);
                    let ink = Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, alpha);
                    let radius = footprint(&h, at, ink);
                    let expected = width * h.state().canvas.shown.pages.first().unwrap().scale as f32 / 2.0;
                    assert!((radius - expected).abs() < 0.01, "{command}, ppp={ppp}, zoom={zoom}: {radius} vs {expected}");
                }
            }
        }
    }
}

#[test]
fn drawing_cursor_leaves_chrome_and_other_tools_their_pointer() {
    let mut h = window(1.0);
    h.state_mut().run("draw.pen", json!({"color": "CC2244", "width": 6.0})).unwrap();
    let at = hover_page(&mut h);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    for point in [pos2(30.0, 20.0), pos2(50.0, at.y)] {
        h.event(egui::Event::PointerMoved(point));
        steps(&mut h, 2);
        assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "outside the page the system pointer remains visible");
    }
    h.event(egui::Event::PointerGone);
    steps(&mut h, 2);
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    hover_page(&mut h);
    h.state_mut().run("draw.select", json!({})).unwrap();
    steps(&mut h, 2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Text);
    h.state_mut().run("draw.lasso", json!({})).unwrap();
    steps(&mut h, 2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Crosshair, "the lasso retains its selection pointer");
    h.state_mut().run("draw.pen", json!({})).unwrap();
    steps(&mut h, 2);
    h.key_press(egui::Key::Escape);
    steps(&mut h, 2);
    assert_eq!(h.state().session.view.draw.mode, wordcraft_engine::cmd::draw::DrawMode::Select);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Text, "Escape restores ordinary text selection");
}

#[test]
fn drawing_cursor_does_not_cover_comment_controls() {
    let mut h = window(1.0);
    h.state_mut().run("select.text", json!({"text": "Drawing"})).unwrap();
    let id = h.state_mut().run("review.newComment", json!({"text": "A cursor comment"})).unwrap()["id"].as_u64().unwrap() as u32;
    h.state_mut().run("view.zoom", json!({"value": 75})).unwrap();
    h.state_mut().run("draw.pen", json!({})).unwrap();
    // The pane and margin balloons are mutually exclusive review surfaces.
    for (pane, active) in [(true, false), (false, false), (false, true)] {
        h.state_mut().run("view.commentsPane", json!({"value": pane})).unwrap();
        if active {
            select_balloon(h.state_mut(), id);
        } else {
            deselect_balloon(h.state_mut());
        }
        steps(&mut h, 4);
        let at = h
            .output()
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(text) if text.galley.text() == "A cursor comment" => Some(text.pos + text.galley.rect.center().to_vec2()),
                _ => None,
            })
            .expect("the comment is visible in the chosen review surface");
        h.event(egui::Event::PointerMoved(at));
        steps(&mut h, 2);
        assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "comments keep their own system pointer");
    }
}

#[test]
fn dark_page_cursor_uses_the_rendered_ink_colour() {
    let mut h = window(1.0);
    h.state_mut().run("view.darkMode", json!({"value": true})).unwrap();
    h.state_mut().run("draw.pen", json!({"color": "000000", "width": 6.0})).unwrap();
    let at = hover_page(&mut h);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    footprint(&h, at, Color32::WHITE);
}

#[test]
fn custom_cursor_keeps_the_drawing_hotspot_and_stroke_eraser_working() {
    let mut h = window(1.0);
    h.state_mut().run("draw.pen", json!({"color": "CC2244", "width": 6.0})).unwrap();
    let at = hover_page(&mut h);
    let button = |h: &mut H, pressed| {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.app_frame();
    };
    button(&mut h, true);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    button(&mut h, false);
    steps(&mut h, 2);
    let strokes = wordcraft_engine::cmd::draw::strokes_in_drawing_order(&mut h.state_mut().session);
    assert_eq!(strokes.len(), 1, "a click still draws at the pointer hotspot");
    let (page, stroke) = &strokes[0];
    assert_eq!(*page, 0);
    let paths = stroke.paths();
    let first = paths.first().and_then(|path| path.first()).unwrap();
    assert!((first[0] - 140.0).abs() < 0.1 && (first[1] - 160.0).abs() < 0.1, "the tool badge never offsets the drawing point");
    h.state_mut().run("draw.eraser", json!({})).unwrap();
    steps(&mut h, 2);
    button(&mut h, true);
    button(&mut h, false);
    assert!(wordcraft_engine::cmd::draw::strokes_in_drawing_order(&mut h.state_mut().session).is_empty());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(wordcraft_engine::cmd::draw::strokes_in_drawing_order(&mut h.state_mut().session).len(), 1);
}

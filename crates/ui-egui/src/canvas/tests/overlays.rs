use craft::pages::ui::PlacedPage;

use super::*;
use crate::canvas::equation_frame::{FrameSizes, equation_frame};
use crate::canvas::gridlines::{self, gridline_cells, gridline_segments};
use crate::table_resize::tests::{H, steps};

/// #1014: Ctrl+drag on a shape leaves it and drops a copy; a plain drag moves it.
#[test]
fn ctrl_drag_copies_an_object() {
    use egui::{Event, Modifiers, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Some text"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    a.run("arrange.bounds", json!({"width": 100, "height": 60, "page": 0, "x": 200, "y": 300})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let objects = |h: &mut H| {
        let doc = &h.state().session.doc;
        doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).map_or(0, |p| p.objects.len())
    };
    let xs = |h: &mut H| {
        let mut v: Vec<i32> = Vec::new();
        for x in (0..600).step_by(50) {
            if h.state_mut().session.layout().find_object(0, |o| (o.rect.x - x as f32).abs() < 0.5).is_some() {
                v.push(x);
            }
        }
        v
    };
    let drag = |h: &mut H, mods: Modifiers| {
        let from = page_to_screen(h.state_mut(), 0, 250.0, 330.0).expect("on screen");
        let to = page_to_screen(h.state_mut(), 0, 400.0, 330.0).expect("on screen");
        h.event(Event::ModifiersChanged(mods));
        h.event(Event::PointerMoved(from));
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
        h.app_frame();
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.app_frame();
        }
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
        for _ in 0..3 {
            h.app_frame();
        }
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.app_frame();
    };
    assert_eq!((objects(&mut h), xs(&mut h)), (1, vec![200]));
    drag(&mut h, Modifiers::COMMAND);
    assert_eq!((objects(&mut h), xs(&mut h)), (2, vec![200, 350]), "the original stays, a copy lands 150pt right");
    // Without Ctrl the object under the pointer (the original) is moved, not copied again:
    // it lands on the copy.
    drag(&mut h, Modifiers::NONE);
    assert_eq!((objects(&mut h), xs(&mut h)), (2, vec![350]));
}

/// Word's frame around the equation being edited: as wide as the equation plus the options
/// strip at its right, as tall as its line, with the grip over its top-left corner. The
/// strip and grip keep their screen size at every zoom.
#[test]
fn the_equation_frame_holds_the_equation_and_its_strip() {
    let math = wordcraft_layout::math::MathLayout { width: 40.0, ascent: 10.0, descent: 5.0, ..Default::default() };
    let bx = wordcraft_layout::hit::EquationBox { page: 0, x: 100.0, base: 90.0, top: 76.0, bottom: 98.0, math: std::sync::Arc::new(math) };
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, 800.0));
    let mut strips = Vec::new();
    for scale in [1.0f32, 2.5] {
        let page = PlacedPage::new(0, rect, f64::from(scale), (0.0, rect.height()));
        // The sizes are screen points the style takes from the tokens, the same at every zoom.
        let sizes = FrameSizes { pad: 2.0, strip: 10.0, grip: vec2(6.0, 12.0) };
        let f = equation_frame(&page, &bx, &sizes);
        let (left, right) = (10.0 + 100.0 * scale, 10.0 + 140.0 * scale);
        assert!(f.outline.min.x < left && f.strip.min.x > right && f.strip.max.x == f.outline.max.x, "{f:?}");
        assert!(f.outline.min.y < 20.0 + 76.0 * scale && f.outline.max.y > 20.0 + 98.0 * scale, "{f:?}");
        assert_eq!((f.strip.min.y, f.strip.max.y), (f.outline.min.y, f.outline.max.y));
        assert!(f.grip.contains(f.outline.left_top() + vec2(1.0, 1.0)), "{f:?}");
        strips.push(f.strip.width());
    }
    assert!((strips[0] - strips[1]).abs() < 1e-3, "the strip keeps its screen size at every zoom: {strips:?}");
}

#[test]
fn floating_picture_can_be_dragged_in_front_of_or_behind_text() {
    for wrap in ["inFrontOfText", "behindText"] {
        drag_floating_picture(wrap, false, true);
    }
}

#[test]
fn floating_header_picture_can_be_dragged_in_front_of_or_behind_text() {
    for wrap in ["inFrontOfText", "behindText"] {
        drag_floating_picture(wrap, true, true);
    }
}

/// A Behind Text picture pressed over the text it sits behind, with no object selected: the
/// press becomes a drag (moving it), while a plain click still edits that text.
#[test]
fn behind_picture_dragged_over_text_without_being_selected_first() {
    drag_floating_picture("behindText", false, false);
    drag_floating_picture("behindText", true, false);
}

fn drag_floating_picture(wrap: &str, header: bool, preselect: bool) {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    if header {
        a.run("insert.editHeader", json!({})).unwrap();
    }
    a.run("text.insert", json!({"text": "The picture overlaps this line of text."})).unwrap();
    let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
    let png = wordcraft_geom::picture::encode_format(&image::DynamicImage::ImageRgba8(img).clone(), image::ImageFormat::Png).unwrap();
    a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 160})).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
    a.run("arrange.bounds", json!({"x": 120, "y": 80})).unwrap();
    a.run("arrange.wrap", json!({"wrap": wrap})).unwrap();
    let before = a.session.layout().object(&pos, 0).unwrap().rect;
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    // The top edge crosses the document's first text line, so Behind Text must still be
    // draggable where its pixels overlap body text. Stay away from resize handles.
    let from = page_to_screen(h.state_mut(), 0, before.x + before.w * 0.25, before.y + 5.0).expect("the page is on screen");
    let to = from + vec2(50.0, 40.0);
    let button = |at, pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    if wrap == "behindText" {
        // A click without drag threshold still reaches the text above the picture.
        h.event(Event::PointerMoved(from));
        h.event(button(from, true));
        h.app_frame();
        h.event(button(from, false));
        for _ in 0..3 {
            h.app_frame();
        }
        // In the header the picture lies over the body area, clear of the header's own text, so a
        // click on it selects it (Word); in the body a click still reaches the text behind it.
        let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).is_some();
        assert_eq!(picked, header, "a click selects the header's picture, but only edits the body's text");
        if preselect {
            let end = Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() };
            h.state_mut().run("select.range", json!({"anchor": pos, "focus": end})).unwrap();
        }
        for _ in 0..40 {
            h.app_frame();
        }
    }
    h.event(Event::PointerMoved(from));
    h.event(button(from, true));
    h.app_frame();
    assert!(h.state().canvas.object_gesture().is_some(), "{wrap}: pressing the image starts a frame drag");
    for k in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
        h.app_frame();
    }
    h.event(button(to, false));
    for _ in 0..3 {
        h.app_frame();
    }
    let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
    let moved = picked.and_then(|p| h.state_mut().session.layout().object(&p, 0)).map(|o| o.rect);
    assert!(moved.is_some_and(|after| (after.x - before.x).abs() > 1.0 || (after.y - before.y).abs() > 1.0), "{wrap}: {before:?} -> {moved:?}");
}

/// 0.5 cm in points.
const HALF_CM: f32 = 72.0 * 0.5 / 2.54;

#[test]
fn table_gridlines_include_header_and_footer_tables() {
    let cell = |y: f32, covered: bool, story: StoryRef| Placed::Cell {
        rect: wordcraft_geom::Rect::new(0.0, y, 10.0, 10.0),
        covered,
        table: Default::default(),
        row: 0,
        cell: 0,
        story,
        clip: false,
    };
    let page = Page {
        items: vec![cell(100.0, false, StoryRef::Body), cell(110.0, true, StoryRef::Body)],
        header: vec![cell(10.0, false, StoryRef::Part(1)), cell(20.0, true, StoryRef::Part(1))],
        footer: vec![cell(200.0, false, StoryRef::Part(2))],
        ..Default::default()
    };
    let ys: Vec<f32> = gridline_cells(&page, true, None).map(|r| r.y).collect();
    assert_eq!(ys, [100.0, 10.0, 200.0]);
    let ys: Vec<f32> = gridline_cells(&page, true, Some(StoryRef::Part(2))).map(|r| r.y).collect();
    assert_eq!(ys, [100.0, 10.0, 200.0], "explicit View Gridlines still shows the other stories");
    let ys = |editing| gridline_cells(&page, false, editing).map(|r| r.y).collect::<Vec<_>>();
    assert_eq!(ys(Some(StoryRef::Part(1))), [10.0]);
    assert_eq!(ys(Some(StoryRef::Part(2))), [200.0]);
    assert!(ys(None).is_empty(), "closing header/footer editing removes the automatic guides");
}

/// #5967: gridlines show only where a cell has no printed border, as in Word; drawn over the
/// border they tinted every bordered table blue.
#[test]
fn table_gridlines_skip_edges_a_border_draws() {
    use wordcraft_geom::Point;
    let cell = Placed::Cell {
        rect: wordcraft_geom::Rect::new(0.0, 0.0, 10.0, 20.0),
        covered: false,
        table: Default::default(),
        row: 0,
        cell: 0,
        story: StoryRef::Body,
        clip: false,
    };
    let rule = |x0, y0, x1, y1, border| Placed::Rule { x0, y0, x1, y1, border };
    let line = wordcraft_doc::props::Border::single(0.5);
    let none = wordcraft_doc::props::Border::single(0.0);
    let page = Page {
        items: vec![
            // Top border over the whole edge (run on past the corners, as layout draws it).
            rule(-0.25, 0.0, 10.25, 0.0, line),
            // Left border over the top half only.
            rule(0.0, 0.0, 0.0, 10.0, line),
            // A zero-width border draws nothing, so the right edge keeps its gridline.
            rule(10.0, 0.0, 10.0, 20.0, none),
            cell,
        ],
        ..Default::default()
    };
    // Overlapping rules out of order on one line leave only the gap between them: the bottom
    // edge keeps 4..6 alone.
    let mut gapped = page.clone();
    gapped.items.extend([
        rule(5.0, 20.0, 10.0, 20.0, line),
        rule(-0.25, 20.0, 3.0, 20.0, line),
        rule(1.0, 20.0, 4.0, 20.0, line),
        rule(6.0, 20.0, 5.5, 20.0, line),
    ]);
    assert_eq!(gridline_segments(&gapped, true, None).first(), Some(&(Point::new(4.0, 20.0), Point::new(5.0, 20.0))));
    assert_eq!(gridline_segments(&gapped, true, None).len(), 3);
    let segs = gridline_segments(&page, true, None);
    assert_eq!(
        segs,
        [
            (Point::new(0.0, 20.0), Point::new(10.0, 20.0)), // bottom: no border
            (Point::new(0.0, 10.0), Point::new(0.0, 20.0)),  // left: below the border
            (Point::new(10.0, 0.0), Point::new(10.0, 20.0)), // right: invisible border
        ]
    );
}

/// #5967: a new table has borders on every edge, so it shows no gridline at all; with its
/// borders removed every edge shows one.
#[test]
fn a_bordered_table_shows_no_gridlines() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("insert.table", json!({"rows": 2, "cols": 3})).unwrap();
    assert!(a.session.view.table_gridlines, "View Gridlines is on by default");
    let segs = |a: &mut WordApp| gridline_segments(&a.session.layout().pages[0], true, None).len();
    assert_eq!(segs(&mut a), 0);
    a.run("table.borders", json!({"kind": "none"})).unwrap();
    assert_eq!(segs(&mut a), 6 * 4, "every edge of the six cells");
}

/// The drawing grid the page shows starts at the text area's top left, stays inside it, spaces
/// its lines by the settings the same both ways or differently across and down, and falls back
/// to the spec's 1/8 inch for a missing spacing.
#[test]
fn drawing_grid_starts_at_the_margins_and_stays_inside_them() {
    // Letter page with 1" margins: the text area is 468 x 648 pt from (72, 72).
    let body = wordcraft_geom::Rect::new(72.0, 72.0, 468.0, 648.0);
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.session.view.gridlines = true;
    let mut page = app.session.layout().pages[0].clone();
    page.body = body;
    // The page hands the framework's `CanvasGrid` its origin, bounds and steps; the grid draws the lines from the
    // origin inside the bounds, one step apart (its own tests).
    for (h, v, want_x, want_y) in
        [(HALF_CM, HALF_CM, HALF_CM, HALF_CM), (9.0, 18.0, 9.0, 18.0), (0.0, f32::NAN, wordcraft_doc::DEFAULT_GRID, wordcraft_doc::DEFAULT_GRID)]
    {
        app.session.doc.settings.grid_h = h;
        app.session.doc.settings.grid_v = v;
        let grid = gridlines::page_grid(&app, &page).expect("the grid shows in Print Layout");
        assert_eq!((grid.origin.x, grid.origin.y), (72.0, 72.0), "origin at the body's top left");
        assert_eq!((grid.bounds.x0, grid.bounds.y0, grid.bounds.x1, grid.bounds.y1), (72.0, 72.0, 540.0, 720.0), "inside the margins");
        assert!((grid.spacing.x - f64::from(want_x)).abs() < 0.01, "across: {:?}", grid.spacing);
        assert!((grid.spacing.y - f64::from(want_y)).abs() < 0.01, "down: {:?}", grid.spacing);
    }
}

#[test]
fn context_menu_crop_item_activates_crop_mode() {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::para::{InlineObject, OBJ};
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let pic = InlineObject::Image {
        media: "m".into(),
        w: 200.0,
        h: 100.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    };
    app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
    let end = Pos::body(0, OBJ.len_utf8());
    app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": end})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let (page, rect) = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).map(|o| (o.page, o.rect)).expect("laid out");
    let at = page_to_screen(h.state_mut(), page, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0).expect("on screen");
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.app_frame();
    }
    h.app_frame();
    assert!(h.state().canvas.context_menu_open, "context menu is open on image");
    // The menu's Crop row (the ribbon above the page has a Crop button of the same name).
    let top = h.state().canvas.page_rects().next().map_or(0.0, |(_, r)| r.min.y);
    let row = h.query_all_by_label("Crop").map(|n| n.rect()).find(|r| r.min.y >= top).map(|r| r.center()).expect("the menu has a Crop row");
    h.event(egui::Event::PointerMoved(row));
    h.app_frame();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: row, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.app_frame();
    }
    for _ in 0..3 {
        h.app_frame();
    }
    assert!(crate::crop::active(h.state()).is_some(), "crop mode is active after clicking Crop in context menu");
}

#[test]
fn crop_is_disabled_when_cursor_is_adjacent_without_selection() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let pic = wordcraft_doc::para::InlineObject::Image {
        media: "m".into(),
        w: 200.0,
        h: 100.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    };
    app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
    // Caret is collapsed right at pos 0 (adjacent to the picture).
    app.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    assert!(crate::ribbon::has_picture_selected(&app.session), "adjacent caret sees picture in ribbon");
    // The state `ui.crop` shows is `can_toggle`'s; running it is refused for the same reason.
    assert!(!crate::crop::can_toggle(&app), "ui.crop is disabled when not selected as object");
    assert!(app.run("ui.crop", json!({})).is_err(), "ui.crop refuses without a selected picture");
    assert!(crate::crop::active(&app).is_none());
    // The command state every surface reads says the same.
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    steps(&mut h, 6);
    assert!(
        !craft::ui::runtime::commands::resolve(&h.ctx, craft::keymap::CommandRef::new("ui.crop")).enabled,
        "the command state of ui.crop is disabled"
    );
}

/// A Behind Text header picture: while editing the header a click on it selects it wherever no
/// header text covers it (far down over the body, or in the band clear of the text), as in Word;
/// over the header text the click edits the text. From the body a click there is body text.
#[test]
fn header_picture_clear_of_header_text_is_selected_while_editing_the_header() {
    // Far down the page, over the body.
    header_picture_click(false, 200.0, 400.0, Some((300.0, f32::MAX)), true);
    // In the header band, below the header's text line.
    header_picture_click(false, 300.0, 56.0, Some((50.0, 72.0)), true);
    // Over the header's text, at its start: the click goes to the text.
    header_picture_click(false, 72.0, 36.0, None, false);
}

/// The same for a footer, which is a `StoryRef::Part` like the header.
#[test]
fn footer_picture_clear_of_footer_text_is_selected_while_editing_the_footer() {
    header_picture_click(true, 200.0, 400.0, Some((300.0, f32::MAX)), true);
}

/// Dragging a header picture that no header text covers moves it, rather than starting the
/// deferred drag-over-text path.
#[test]
fn dragging_a_header_picture_clear_of_header_text_moves_it() {
    let (mut h, pos, rect) = header_picture_app(false, 200.0, 400.0, 1);
    edit_story(&mut h, false, 0);
    let at = picture_point(&mut h, 0, rect);
    h.event(egui::Event::PointerMoved(at));
    h.event(pointer_button(at, egui::PointerButton::Primary, true));
    h.app_frame();
    let to = at + egui::vec2(60.0, 30.0);
    for p in [at + egui::vec2(20.0, 10.0), to] {
        h.event(egui::Event::PointerMoved(p));
        h.app_frame();
    }
    h.event(pointer_button(to, egui::PointerButton::Primary, false));
    steps(&mut h, 3);
    let moved = h.state_mut().session.layout().object(&pos, 0).unwrap().rect;
    assert!(moved.x > rect.x + 10.0 && moved.y > rect.y + 5.0, "the picture moved with the drag: {rect:?} -> {moved:?}");
    assert_eq!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p), Some(pos), "and stays selected");
}

/// A right-click on the header picture repeated on page 2 while editing the header on page 1
/// selects that copy: the frame and handles belong on page 2, not the page last edited.
#[test]
fn right_clicking_a_repeated_header_picture_selects_it_on_its_own_page() {
    let (mut h, pos, _) = header_picture_app(false, 200.0, 400.0, 2);
    edit_story(&mut h, false, 0);
    assert_eq!(h.state().session.page_hint, 0, "editing the header on page 1");
    let rect = h.state_mut().session.layout().object(&pos, 1).unwrap().rect;
    assert_eq!(h.state_mut().session.layout().object(&pos, 1).unwrap().page, 1, "the picture is repeated on page 2");
    let at = picture_point(&mut h, 1, rect);
    press(&mut h, at, egui::PointerButton::Secondary);
    assert_eq!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p), Some(pos), "the right-click selected it");
    assert_eq!(h.state().session.page_hint, 1, "its frame is drawn on the page clicked");
}

/// The same for a left-click: it selects the page-2 copy and moves the frame there, not the copy
/// on page 1 that was being edited.
#[test]
fn left_clicking_a_repeated_header_picture_selects_it_on_its_own_page() {
    let (mut h, pos, _) = header_picture_app(false, 200.0, 400.0, 2);
    edit_story(&mut h, false, 0);
    assert_eq!(h.state().session.page_hint, 0, "editing the header on page 1");
    let rect = h.state_mut().session.layout().object(&pos, 1).unwrap().rect;
    assert_eq!(h.state_mut().session.layout().object(&pos, 1).unwrap().page, 1, "the picture is repeated on page 2");
    let at = picture_point(&mut h, 1, rect);
    press(&mut h, at, egui::PointerButton::Primary);
    assert_eq!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p), Some(pos), "the left-click selected it");
    assert_eq!(h.state().session.page_hint, 1, "its frame is drawn on the page clicked");
}

fn pointer_button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() }
}

/// Moves to `at`, presses and releases `button` there and lets the frames settle.
fn press(h: &mut H, at: egui::Pos2, button: egui::PointerButton) {
    h.event(egui::Event::PointerMoved(at));
    h.event(pointer_button(at, button, true));
    h.app_frame();
    h.event(pointer_button(at, button, false));
    steps(h, 3);
}

/// Near the top-left corner of the picture's `rect` on `page`: on the header text line when the
/// picture is placed over it.
fn picture_point(h: &mut H, page: usize, rect: wordcraft_geom::Rect) -> egui::Pos2 {
    page_to_screen(h.state_mut(), page, rect.x + 10.0, rect.y + 6.0).expect("the page is on screen")
}

/// Starts editing the header (or footer) of `page` and lets the frames settle.
fn edit_story(h: &mut H, footer: bool, page: usize) {
    let command = if footer { "insert.editFooter" } else { "insert.editHeader" };
    h.state_mut().run(command, json!({"page": page})).unwrap();
    // Entering the footer scrolls the page to it: wait until the page stops moving.
    let first = |h: &H| h.state().canvas.page_rects().next();
    let (mut last, mut still) = (first(h), 0);
    for _ in 0..600 {
        h.app_frame();
        let now = first(h);
        still = if now == last { still + 1 } else { 0 };
        last = now;
        if still >= 10 {
            break;
        }
    }
}

/// A body-mode window on a document of `pages` pages whose header (or footer) holds text and a
/// 160pt Behind Text picture at (`x`, `y`) on the page. Returns it, the picture's position and
/// its first-page rectangle.
fn header_picture_app(footer: bool, x: f32, y: f32, pages: usize) -> (H, Pos, wordcraft_geom::Rect) {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    for _ in 1..pages {
        a.run("insert.pageBreak", json!({})).unwrap();
    }
    a.run(if footer { "insert.editFooter" } else { "insert.editHeader" }, json!({})).unwrap();
    a.run("text.insert", json!({"text": if footer { "Footer" } else { "Header" }})).unwrap();
    let mut png = Vec::new();
    let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 160})).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
    a.run("arrange.bounds", json!({"x": x, "y": y})).unwrap();
    a.run("arrange.wrap", json!({"wrap": "behindText"})).unwrap();
    let rect = a.session.layout().object(&pos, 0).unwrap().rect;
    a.run("insert.closeHeader", json!({})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    // Zoomed out so the whole page is on screen and entering the footer scrolls nothing.
    h.state_mut().run("view.zoom", json!({"value": 40})).unwrap();
    steps(&mut h, 20);
    (h, pos, rect)
}

/// Puts a Behind Text header (or footer) picture at (`x`, `y`), checks its top lies within `band`
/// (when given), clicks near its top-left corner from the body and then while editing, and checks
/// the second click selects it (`selects`) or leaves the caret in the text; a right-click picks it
/// only where the click does, so the context menu (Reset Picture, Wrap Text) acts on it.
fn header_picture_click(footer: bool, x: f32, y: f32, band: Option<(f32, f32)>, selects: bool) {
    let (mut h, pos, rect) = header_picture_app(footer, x, y, 1);
    if let Some((lo, hi)) = band {
        assert!(rect.y >= lo && rect.y < hi, "the picture sits where the case wants it: {rect:?}");
    }
    let at = picture_point(&mut h, 0, rect);
    let picked = |h: &H| wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
    press(&mut h, at, egui::PointerButton::Primary);
    assert!(picked(&h).is_none(), "from the body the picture isn't selectable");
    assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
    edit_story(&mut h, footer, 0);
    press(&mut h, at, egui::PointerButton::Primary);
    if selects {
        assert_eq!(picked(&h), Some(pos.clone()), "while editing a click on its picture selects it ({x}, {y})");
    } else {
        assert_eq!(picked(&h), None, "over the text the click edits the text ({x}, {y})");
        assert!(matches!(h.state().session.sel.focus.story, StoryRef::Part(_)), "still in the header or footer");
    }
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(Pos { off: 0, ..pos.clone() });
    press(&mut h, at, egui::PointerButton::Secondary);
    assert_eq!(picked(&h), selects.then(|| pos.clone()), "a right-click picks the picture only where a click does ({x}, {y})");
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.app_frame();
}

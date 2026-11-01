use super::*;

#[test]
fn highlight_button_appearance_follows_the_picked_colour() {
    assert_eq!(highlight_button_appearance("cyan"), ("highlight", Some(egui::Color32::from_rgb(0, 255, 255)), "cyan"));
    assert_eq!(highlight_button_appearance("none"), ("highlightNoColor", None, "none"));
    assert_eq!(highlight_button_appearance("unknown"), ("highlightNoColor", None, "none"));
}

fn vp() -> Rect {
    Rect::from_min_size(pos2(100.0, 50.0), Vec2::new(1200.0, 700.0))
}

#[test]
fn reading_mode_hides_formatting_even_with_an_existing_selection() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("text.insert", json!({"text": "Selectable text"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    assert!(!app.session.sel.is_collapsed());
    assert!(visible(&app), "selection shows the formatting bar in editing mode");

    app.run("view.readMode", json!({})).unwrap();
    assert!(!app.session.sel.is_collapsed(), "entering Reading Mode must preserve the selection");
    assert!(!visible(&app), "formatting bar must not be accessible in Reading Mode");

    app.run("view.printLayout", json!({})).unwrap();
    assert!(visible(&app), "returning to editing restores the formatting bar");
}

#[test]
fn sits_above_the_selection_when_there_is_room() {
    let left = vp().center().x;
    let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
    assert_eq!(r.size(), Vec2::new(BAR_W, BAR_H));
    assert!((r.max.y - (400.0 - VGAP)).abs() < 1e-3, "bar bottom should clear the selection by VGAP");
    assert!(r.max.y < 400.0, "the bar must be above the selection");
    assert!((r.center().x - (left + ANCHOR_INSET)).abs() < 1e-3, "bar should be centred on the selection's left edge");
}

#[test]
fn centred_unless_the_viewport_forces_a_shift() {
    let left = vp().min.x + 20.0;
    let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
    assert!((r.min.x - vp().min.x).abs() < 1e-3, "the bar must be pushed onto the viewport's left edge");
    let right = vp().max.x - 20.0;
    let r = anchor_rect(400.0, right, vp()).unwrap_or(Rect::EVERYTHING);
    assert!((r.max.x - vp().max.x).abs() < 1e-3, "the bar must be pushed onto the viewport's right edge");
}

#[test]
fn flips_below_when_there_is_no_room_above() {
    let r = anchor_rect(60.0, 300.0, vp()).unwrap_or(Rect::EVERYTHING);
    assert!((r.min.y - (60.0 + VGAP)).abs() < 1e-3, "bar top should clear the selection by VGAP below");
    assert!(r.min.y > 60.0, "the bar must be below the selection");
}

#[test]
fn never_leaves_the_viewport() {
    for left in [0.0, 130.0, 400.0, 2000.0, -500.0] {
        let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
        assert!(r.min.x >= vp().min.x - 1e-3, "left {left}: bar left {} out of viewport", r.min.x);
        assert!(r.max.x <= vp().max.x + 1e-3, "left {left}: bar right {} out of viewport", r.max.x);
    }
}

#[test]
fn vertical_position_stays_inside_viewport_for_offscreen_anchor() {
    let viewport = vp();
    for selection_top in [viewport.top() - 500.0, viewport.bottom() + 500.0] {
        let r = anchor_rect(selection_top, viewport.center().x, viewport).unwrap_or(Rect::EVERYTHING);
        assert!(r.min.y >= viewport.top() - 1e-3, "bar top {} is above viewport {}", r.min.y, viewport.top());
        assert!(r.max.y <= viewport.bottom() + 1e-3, "bar bottom {} exceeds viewport {}", r.max.y, viewport.bottom());
    }
}

#[test]
fn anchor_skips_rects_scrolled_offscreen_and_clips_partial_ones() {
    let clip = vp();
    // Page 1's line is above the viewport, page 2's line is inside it.
    let above = Rect::from_min_size(pos2(100.0, clip.top() - 300.0), Vec2::new(200.0, 20.0));
    let inside = Rect::from_min_size(pos2(120.0, clip.top() + 50.0), Vec2::new(200.0, 20.0));
    assert_eq!(visible_anchor([above, inside], clip), Some(inside));
    // Partly scrolled off the top: anchored to the visible part, never above the viewport.
    let partial = Rect::from_min_size(pos2(100.0, clip.top() - 10.0), Vec2::new(200.0, 20.0));
    let a = visible_anchor([partial], clip).unwrap();
    assert!((a.min.y - clip.top()).abs() < 1e-3 && (a.height() - 10.0).abs() < 1e-3);
    // Entirely off either end, or only touching the edge: nothing to anchor to.
    let below = Rect::from_min_size(pos2(100.0, clip.bottom() + 5.0), Vec2::new(200.0, 20.0));
    let touching = Rect::from_min_size(pos2(100.0, clip.top() - 20.0), Vec2::new(200.0, 20.0));
    assert_eq!(visible_anchor([above, below, touching], clip), None);
    assert_eq!(visible_anchor([], clip), None);
    // Input order does not matter: the topmost visible rectangle wins.
    let lower = Rect::from_min_size(pos2(100.0, clip.top() + 200.0), Vec2::new(200.0, 20.0));
    assert_eq!(visible_anchor([lower, inside], clip), Some(inside));
    // Degenerate input is rejected, not propagated.
    let nan = Rect::from_min_size(pos2(f32::NAN, 0.0), Vec2::new(10.0, 10.0));
    assert_eq!(visible_anchor([nan], clip), None);
}

#[test]
fn bar_for_a_partly_hidden_selection_stays_clear_of_the_top_edge() {
    let clip = vp();
    let partial = Rect::from_min_size(pos2(100.0, clip.top() - 40.0), Vec2::new(200.0, 20.0 + 30.0));
    let a = visible_anchor([partial], clip).unwrap();
    let bar = anchor_rect(a.min.y, a.min.x, clip).unwrap();
    assert!(bar.min.y >= clip.top() - 1e-3 && bar.max.y <= clip.bottom() + 1e-3);
}

#[test]
fn degenerate_inputs_are_rejected_not_crashed() {
    assert!(anchor_rect(f32::NAN, 10.0, vp()).is_none());
    assert!(anchor_rect(10.0, f32::NAN, vp()).is_none());
    assert!(anchor_rect(f32::INFINITY, 10.0, vp()).is_none());
    assert!(anchor_rect(10.0, f32::NEG_INFINITY, vp()).is_none());
    assert!(anchor_rect(10.0, 10.0, Rect::EVERYTHING).is_none());
    assert!(anchor_rect(10.0, 10.0, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(0.0, 100.0))).is_none());
}

#[test]
fn a_viewport_narrower_than_the_bar_still_yields_a_rect() {
    let r = anchor_rect(10.0, 5.0, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(20.0, 20.0)));
    let r = r.unwrap_or(Rect::EVERYTHING);
    assert!(r.width() > 0.0 && r.height() > 0.0, "a tiny viewport must still place a positive-size bar");
    assert!(r.min.x.is_finite() && r.min.y.is_finite());
}

/// #535: the bar was one fixed-width row that cut off everything after Align Left. Draw it
/// over a real selection: every control fits inside the bar, and the bar is no wider than its
/// controls (both rows end together).
#[test]
fn every_control_fits_inside_the_bar() {
    use wordcraft_engine::Session;
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "The quick brown fox jumps over the lazy dog"})).unwrap();
    a.run("select.text", json!({"text": "brown fox"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(1440.0, 900.0)).build_ui_state(
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
    let [bar, used] = h.ctx.data(|d| d.get_temp::<[Rect; 2]>(shown_id())).expect("the bar is shown over a selection");
    let viewport = h.state().canvas.viewport.expect("the canvas exposes its actual scroll viewport");
    assert!(viewport.contains_rect(bar), "toolbar {:?} must stay inside canvas viewport {:?}", bar, viewport);
    let inner = bar.shrink2(Vec2::new(PAD_X, PAD_Y));
    assert!(
        inner.expand(0.1).contains_rect(used),
        "controls {:?} spill out of the bar {:?}",
        [used.min.x, used.min.y, used.max.x, used.max.y],
        [inner.min.x, inner.min.y, inner.max.x, inner.max.y]
    );
    assert!((used.width() - inner.width()).abs() < 0.5, "the bar ({}) should be as wide as its controls ({})", inner.width(), used.width());
    assert!(used.min.x - inner.min.x < 0.5, "the font box starts at the bar's left edge");
}

/// The whole window over a document with `pick` selected, run until the bar is up.
fn bar_over(pick: &str) -> egui_kittest::Harness<'static, WordApp> {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "The quick brown fox jumps over the lazy dog"})).unwrap();
    a.run("select.text", json!({"text": pick})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h);
    h
}

fn steps(h: &mut egui_kittest::Harness<'static, WordApp>) {
    for _ in 0..6 {
        h.step();
    }
}

/// The bar's layer, when it was drawn last frame.
fn bar_layer(h: &egui_kittest::Harness<'static, WordApp>) -> Option<egui::LayerId> {
    h.ctx.memory(|m| m.areas().visible_layer_ids().into_iter().find(|l| l.id == egui::Id::unique("wc_mini_toolbar")))
}

fn click_at(h: &mut egui_kittest::Harness<'static, WordApp>, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    steps(h);
}

fn format_state(h: &mut egui_kittest::Harness<'static, WordApp>) -> Value {
    h.state_mut().session.run("format.state", &json!({})).unwrap()
}

/// #2959: formatting must be available alongside the text's right-click menu, using the
/// selection that was clicked. Its lists must remain usable inside the context popup.
#[test]
fn context_menu_toolbar_formats_the_selection_and_opens_its_size_list() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut h = bar_over("brown fox");
    let selection = h.state().session.sel.clone();
    let caret = h.state_mut().session.layout().caret(&wordcraft_doc::Pos::body(0, 12)).unwrap();
    let at = crate::canvas::page_to_screen(h.state_mut(), caret.page, caret.x, caret.top + caret.height / 2.0).unwrap();
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    steps(&mut h);
    assert!(h.state().canvas.context_menu_open);
    assert_eq!(h.state().session.sel, selection, "right-click inside the selection keeps it");
    assert!(bar_layer(&h).is_none(), "the standalone selection toolbar must not double up");
    let viewport = h.state().canvas.viewport.unwrap();
    let bold = h.get_all_by_label("Bold").map(|n| n.rect()).find(|r| viewport.contains_rect(*r)).expect("the context menu has Bold");
    click_at(&mut h, bold.center());
    assert_eq!(format_state(&mut h).get("bold"), Some(&json!(true)));
    assert_eq!(h.state().session.sel, selection);
    assert!(h.state().canvas.context_menu_open, "formatting keeps the context toolbar available");

    let size_box = h
        .get_all_by_role(Role::TextInput)
        .map(|n| n.rect())
        .filter(|r| viewport.contains_rect(*r))
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .expect("the context toolbar has a size box");
    click_at(&mut h, pos2(size_box.max.x + 8.0, size_box.center().y));
    let twenty = h.get_by_label("20").rect().center();
    click_at(&mut h, twenty);
    assert_eq!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(20.0));
    assert_eq!(h.state().session.sel, selection);
    assert!(h.state().canvas.context_menu_open, "picking a font size keeps the parent menu available");
    click_at(&mut h, pos2(size_box.max.x + 8.0, size_box.center().y));
    assert!(egui::Popup::is_any_open(&h.ctx), "the child list is open");
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    steps(&mut h);
    assert!(!egui::Popup::is_any_open(&h.ctx), "Escape closes the child list");
    assert!(h.state().canvas.context_menu_open, "the parent toolbar remains after dismissing its list");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_ne!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(20.0), "size is one undo step");
    assert_eq!(format_state(&mut h).get("bold"), Some(&json!(true)), "the earlier Bold is a separate undo step");
    let copy = h.query_all_by_label_contains("Copy").map(|n| n.rect()).find(|r| viewport.contains_rect(*r)).expect("the menu has Copy");
    click_at(&mut h, copy.center());
    assert!(!h.state().canvas.context_menu_open, "ordinary menu commands still close the popup");
}

#[test]
fn context_menu_toolbar_opens_at_a_caret_from_the_keyboard_and_dismisses() {
    use egui_kittest::kittest::Queryable;
    let mut h = bar_over("brown fox");
    h.state_mut().run("caret.docEnd", json!({})).unwrap();
    steps(&mut h);
    let caret = h.state().session.sel.clone();
    h.event(egui::Event::Key { key: egui::Key::F10, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT });
    steps(&mut h);
    let viewport = h.state().canvas.viewport.unwrap();
    assert!(h.state().canvas.context_menu_open);
    let bold = h.get_all_by_label("Bold").map(|n| n.rect()).find(|r| viewport.contains_rect(*r)).expect("the keyboard menu has Bold at a caret too");
    click_at(&mut h, bold.center());
    assert_eq!(format_state(&mut h).get("bold"), Some(&json!(true)));
    assert_eq!(h.state().session.sel, caret);
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    steps(&mut h);
    assert!(!h.state().canvas.context_menu_open);
    h.event(egui::Event::Text(" typed".into()));
    steps(&mut h);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).ends_with(" typed"), "typing resumes in the document");

    h.state_mut().run("ui.contextMenu", json!({})).unwrap();
    steps(&mut h);
    assert!(h.state().canvas.context_menu_open);
    click_at(&mut h, viewport.right_bottom() - Vec2::splat(10.0));
    assert!(!h.state().canvas.context_menu_open, "an outside click dismisses both panels");
}

#[test]
fn context_menu_toolbar_fits_near_the_bottom_right_of_a_narrow_window() {
    use egui_kittest::kittest::Queryable;
    let mut h = bar_over("brown fox");
    h.set_size(Vec2::new(380.0, 650.0));
    steps(&mut h);
    h.state_mut().run("ui.contextMenu", json!({})).unwrap();
    steps(&mut h);
    h.state_mut().canvas.context_menu_at = Some(pos2(370.0, 640.0));
    steps(&mut h);
    let screen = h.ctx.content_rect();
    let bold = h.get_all_by_label("Bold").map(|n| n.rect()).find(|r| r.min.y > 200.0).expect("the context toolbar is shown");
    assert!(screen.contains_rect(bold), "the toolbar stays inside the window: {bold:?} in {screen:?}");
    let copy = h.query_all_by_label_contains("Copy").map(|n| n.rect()).find(|r| r.min.y > 200.0).expect("the menu has Copy");
    assert!(screen.contains_rect(copy), "the menu stays inside the window: {copy:?} in {screen:?}");
    let align = h.get_all_by_label("Justify").map(|n| n.rect()).find(|r| r.min.y > 200.0).expect("the last toolbar control is shown");
    assert!(screen.contains_rect(align), "all controls fit: {align:?} in {screen:?}");
}

#[test]
fn context_menu_toolbar_respects_read_mode_and_the_toolbar_preference() {
    use egui_kittest::kittest::Queryable;
    for read_mode in [false, true] {
        let mut h = bar_over("brown fox");
        h.state_mut().session.view.read_mode = read_mode;
        h.state_mut().ui.mini_toolbar = read_mode;
        h.state_mut().run("ui.contextMenu", json!({})).unwrap();
        steps(&mut h);
        assert!(h.state().canvas.context_menu_open, "the text menu remains available");
        let top = h.state().canvas.viewport.unwrap().top();
        assert!(h.query_all_by_label("Bold").all(|n| n.rect().min.y < top), "the context toolbar is suppressed, read_mode={read_mode}");
        assert!(h.query_all_by_label_contains("Copy").any(|n| n.rect().min.y > top), "the menu still offers Copy");
    }
}

/// The command panel under the context toolbar was stretched to the toolbar's width; each
/// keeps its own width, so the panel matches the menu shown without the toolbar.
#[test]
fn context_menu_panel_keeps_its_own_width_under_the_toolbar() {
    use egui_kittest::kittest::Queryable;
    let copy_width = |toolbar: bool| {
        let mut h = bar_over("brown fox");
        h.state_mut().ui.mini_toolbar = toolbar;
        h.state_mut().run("ui.contextMenu", json!({})).unwrap();
        steps(&mut h);
        let top = h.state().canvas.viewport.unwrap().top();
        let copy = h.query_all_by_label_contains("Copy").map(|n| n.rect()).find(|r| r.min.y > top).expect("the menu has Copy");
        let bar = h.query_all_by_label("Justify").map(|n| n.rect()).find(|r| r.min.y > top);
        (copy, bar)
    };
    let (plain, _) = copy_width(false);
    let (copy, justify) = copy_width(true);
    let justify = justify.expect("the context toolbar is shown");
    assert!((copy.width() - plain.width()).abs() < 0.5, "panel {} vs plain menu {}", copy.width(), plain.width());
    assert!(copy.max.x + 20.0 < justify.max.x, "the panel ({copy:?}) is narrower than the toolbar ({justify:?})");
}

#[test]
fn escape_in_a_ribbon_list_does_not_open_the_context_menu() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut h = bar_over("brown fox");
    let size = h
        .get_all_by_role(Role::TextInput)
        .filter(|n| n.value().as_deref() == Some("12"))
        .map(|n| n.rect())
        .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .expect("the ribbon has a size field");
    click_at(&mut h, pos2(size.max.x + 8.0, size.center().y));
    assert!(egui::Popup::is_any_open(&h.ctx));
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    steps(&mut h);
    assert!(!egui::Popup::is_any_open(&h.ctx));
    assert!(!h.state().canvas.context_menu_open, "dismissing another popup must not open the text context menu");
}

/// The bar was drawn on the Tooltip layer, over every drop-down: the ribbon's font size list
/// opened under it. Popups and menus are on Foreground; the bar must sit below them.
#[test]
fn the_bar_draws_below_popups() {
    let h = bar_over("brown fox");
    let layer = bar_layer(&h).expect("the bar is shown over a selection");
    assert!(layer.order < Order::Foreground, "the bar is on {:?}, over the ribbon's drop-downs", layer.order);
}

/// Turning to the ribbon puts the bar away (it covered the ribbon's drop-downs); it comes back
/// for the next selection, even one over the same text.
#[test]
fn a_click_on_the_ribbon_puts_the_bar_away_until_the_selection_changes() {
    use egui_kittest::kittest::Queryable;
    let mut h = bar_over("brown fox");
    assert!(visible(h.state()) && bar_layer(&h).is_some(), "the bar is up over the selection");
    // The ribbon's Bold is the topmost one; the bar has its own lower down.
    let ribbon_bold = |h: &egui_kittest::Harness<'static, WordApp>| {
        h.get_all_by_label("Bold").map(|n| n.rect().center()).min_by(|a, b| a.y.total_cmp(&b.y)).expect("the ribbon has Bold")
    };
    let at = ribbon_bold(&h);
    click_at(&mut h, at);
    assert_eq!(format_state(&mut h).get("bold"), Some(&json!(true)), "the click ran the ribbon's Bold");
    assert!(!visible(h.state()), "a click on the ribbon puts the bar away");
    assert!(bar_layer(&h).is_none(), "the bar is no longer drawn");

    h.state_mut().run("select.text", json!({"text": "lazy"})).unwrap();
    steps(&mut h);
    assert!(visible(h.state()) && bar_layer(&h).is_some(), "a new selection brings the bar back");

    let at = ribbon_bold(&h);
    click_at(&mut h, at);
    assert!(bar_layer(&h).is_none());
    h.state_mut().run("select.text", json!({"text": "quick"})).unwrap();
    steps(&mut h);
    h.state_mut().run("select.text", json!({"text": "lazy"})).unwrap();
    steps(&mut h);
    assert!(bar_layer(&h).is_some(), "selecting the same text again brings the bar back");
}

/// The ribbon's size list hangs over the page: picking from it is no press in the document, so
/// the bar stays away (it would cover the list again otherwise).
#[test]
fn picking_from_a_ribbon_list_over_the_page_keeps_the_bar_away() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut h = bar_over("brown fox");
    let size = format_state(&mut h).get("size").and_then(Value::as_f64).unwrap_or_default();
    let size = if size.fract() == 0.0 { format!("{size:.0}") } else { size.to_string() };
    // The ribbon's size box is the topmost one showing the size; the bar has its own.
    let size_box = h.get_all_by_role(Role::TextInput).filter(|n| n.value().as_deref() == Some(size.as_str())).map(|n| n.rect());
    let size_box = size_box.min_by(|a, b| a.min.y.total_cmp(&b.min.y)).expect("the ribbon has a size box");
    click_at(&mut h, pos2(size_box.max.x + 8.0, size_box.center().y));
    assert!(bar_layer(&h).is_none(), "opening the ribbon's list puts the bar away");
    let page = h.state().canvas.canvas_rect.expect("the document is shown");
    let item = h.get_by_label("28").rect().center();
    assert!(page.contains(item), "the list's 28 ({item:?}) should hang over the page ({page:?}) for this test");
    click_at(&mut h, item);
    assert_eq!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(28.0), "the ribbon's size list applied 28 pt");
    assert!(!visible(h.state()) && bar_layer(&h).is_none(), "the bar stays away after a pick from the ribbon's list");
}

/// Scrolled until the selection went under the ribbon, the bar went with it and drew over the
/// ribbon and the ruler (#1069). With no part of the selection on screen it has no anchor
/// (`visible_anchor`, #1501), so it hides: nothing over the ruler, which still takes the drag.
#[test]
// #1069 lets the bar slide under the ribbon with a selection scrolled past the document's
// top; #1501, landed first, anchors it to the visible part of the selection only and drops it
// once none shows. Which one Word does is the owner's call; until then this one is off.
#[ignore = "conflicts with #1501's visible-selection anchor; see the comment above"]
fn the_bar_scrolls_under_the_ribbon_not_over_it() {
    let mut h = bar_over("The quick");
    let page = h.state().canvas.canvas_rect.expect("the document is shown");
    // Scroll the selection's line to 45 points above the document area, under the ruler.
    let target = page.min.y - 45.0;
    let line = h.state().canvas.mini_anchor.expect("the selection is on screen");
    h.event(egui::Event::PointerMoved(page.center()));
    h.step();
    let delta = egui::vec2(0.0, target - line.min.y);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Start,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    });
    h.step();
    h.state_mut().canvas.wheel = Default::default();
    steps(&mut h);
    assert!(h.state().canvas.mini_anchor.is_none(), "the selection scrolled off the document area");
    assert!(bar_layer(&h).is_none(), "the bar hides with its selection, not over the ribbon");

    // The ruler's left-indent marker sits at the line's start and the ruler's foot
    // (`canvas::rulers`); dragging it moves the indent.
    let marker = pos2(line.min.x, page.min.y - 6.0);
    h.event(egui::Event::PointerMoved(marker));
    h.event(egui::Event::PointerButton { pos: marker, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for dx in [6.0, 18.0, 36.0] {
        h.event(egui::Event::PointerMoved(marker + egui::vec2(dx, 0.0)));
        h.step();
    }
    let end = marker + egui::vec2(36.0, 0.0);
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    steps(&mut h);
    let s = &h.state().session;
    let indent = s.doc.para_at(&s.sel.focus).map(|p| s.doc.styles.resolve_para(&p.props).indent_left).unwrap_or_default();
    assert!(indent > 0.0, "dragging the ruler's left-indent marker where the bar was left the indent at {indent}");
}

/// Picking from one of the bar's own lists (popups below it) keeps the bar up.
#[test]
fn picking_from_the_bars_own_list_keeps_it() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut h = bar_over("brown fox");
    let [bar, _] = h.ctx.data(|d| d.get_temp::<[Rect; 2]>(shown_id())).expect("the bar is shown over a selection");
    // Its two boxes are font and size; the size box's arrow is just right of the rightmost.
    let size_box = h.get_all_by_role(Role::TextInput).map(|n| n.rect()).filter(|r| bar.contains_rect(*r)).max_by(|a, b| a.min.x.total_cmp(&b.min.x));
    let size_box = size_box.expect("the bar has a size box");
    click_at(&mut h, pos2(size_box.max.x + 8.0, size_box.center().y));
    let twenty = h.get_by_label("20").rect().center();
    click_at(&mut h, twenty);
    assert_eq!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(20.0), "the bar's size list applied 20 pt");
    assert!(visible(h.state()) && bar_layer(&h).is_some(), "the bar stays up after its own list is used");
}

/// #2105: No Color removed the highlight but the Highlight button kept the colour picked
/// before, so the next click on it highlighted again. As in Word, No Color becomes the
/// button's choice: the next click removes highlighting.
#[test]
fn no_color_becomes_the_highlight_buttons_choice() {
    use egui_kittest::kittest::{By, Queryable};
    let mut h = bar_over("brown fox");
    h.state_mut().run("format.highlight", json!({"color": "yellow"})).unwrap();
    steps(&mut h);
    // The bar's split buttons are Highlight then Font Color; their arrows are 11 × 22.
    let highlight_arrow = |h: &egui_kittest::Harness<'static, WordApp>| {
        let [bar, _] = h.ctx.data(|d| d.get_temp::<[Rect; 2]>(shown_id())).expect("the bar is shown over a selection");
        h.query_all(By::new().predicate(|n| n.raw_bounds().is_some_and(|r| ((r.x1 - r.x0) - 11.0).abs() < 0.5 && ((r.y1 - r.y0) - 22.0).abs() < 0.5)))
            .map(|n| n.rect())
            .filter(|r| bar.contains_rect(*r))
            .min_by(|a, b| a.min.x.total_cmp(&b.min.x))
            .expect("the bar has the Highlight split button")
    };
    let arrow = highlight_arrow(&h);
    click_at(&mut h, arrow.center());
    let no_color = h.get(By::new().label_contains("No Color")).rect().center();
    click_at(&mut h, no_color);
    assert_eq!(format_state(&mut h).get("highlight"), Some(&Value::Null), "No Color removed the highlight");
    assert_eq!(h.state().canvas.last_highlight, "none");

    // Other highlighted text, then the button itself (left of its arrow): no highlight.
    h.state_mut().run("select.text", json!({"text": "lazy"})).unwrap();
    h.state_mut().run("format.highlight", json!({"color": "yellow"})).unwrap();
    steps(&mut h);
    let arrow = highlight_arrow(&h);
    click_at(&mut h, pos2(arrow.min.x - 12.0, arrow.center().y));
    assert_eq!(format_state(&mut h).get("highlight"), Some(&Value::Null), "the button applies No Color, not yellow again");

    // A stale or corrupted remembered value looks like No Color and must act like it too.
    h.state_mut().canvas.last_highlight = "unknown".into();
    h.state_mut().run("format.highlight", json!({"color": "yellow"})).unwrap();
    steps(&mut h);
    let arrow = highlight_arrow(&h);
    click_at(&mut h, pos2(arrow.min.x - 12.0, arrow.center().y));
    assert_eq!(format_state(&mut h).get("highlight"), Some(&Value::Null), "an unknown remembered colour is normalized to No Color");
}

/// #3039: picking Automatic from the bar's font-colour menu hands the keyboard back to the
/// document.
#[test]
fn the_font_colour_menus_automatic_returns_focus_to_the_document() {
    use egui_kittest::kittest::{By, Queryable};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), Default::default());
    a.run("select.all", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(400.0, 400.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            egui::Area::new(egui::Id::unique("font_colour_probe")).fixed_pos(pos2(50.0, 50.0)).show(ui.ctx(), |ui| {
                font_color_menu(ui, app);
            });
        },
        a,
    );
    steps(&mut h);
    let automatic = h.get(By::new().label_contains("Automatic")).rect().center();
    h.state_mut().canvas.want_focus = false;
    click_at(&mut h, automatic);
    assert!(h.state().canvas.want_focus, "Automatic returns focus to the document");
}

/// #3038: a pick from the bar's highlight grid hands the keyboard back to the document,
/// like the No Color button beside the swatches (it ran through `run_from_ribbon` already).
#[test]
fn picking_a_highlight_swatch_returns_focus_to_the_document() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), Default::default());
    a.run("select.all", json!({})).unwrap();
    let origin = egui::Id::unique("hl_swatch_origin");
    let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(400.0, 300.0)).build_ui_state(
        move |ui, app: &mut WordApp| {
            egui::Area::new(egui::Id::unique("hl_probe")).fixed_pos(pos2(100.0, 100.0)).show(ui.ctx(), |ui| {
                ui.ctx().data_mut(|d| d.insert_temp(origin, ui.next_widget_position()));
                highlight_menu(ui, app);
            });
        },
        a,
    );
    steps(&mut h);
    // The first rendered swatch is Yellow (None is skipped), 18 × 18 at the grid's origin.
    let first = h.ctx.data(|d| d.get_temp::<egui::Pos2>(origin)).expect("the grid drew");
    h.state_mut().canvas.want_focus = false;
    click_at(&mut h, first + Vec2::new(9.0, 9.0));
    assert!(h.state().canvas.want_focus, "the swatch pick returns focus to the document");
    assert!(format_state(&mut h).get("highlight").is_some_and(|v| !v.is_null()), "the first swatch applied a highlight");
}

/// #3579: in a right-to-left paragraph the bar lit Align Left for a paragraph that sits on
/// the right. Like the ribbon, it marks the alignment as seen on the page, the one its buttons
/// set. Only the bar is drawn, so the ribbon's own alignment buttons can't answer for it.
#[test]
fn alignment_buttons_mark_the_alignment_seen_on_the_page_in_a_right_to_left_paragraph() {
    use egui_kittest::kittest::Queryable;
    // A left-to-right paragraph first: a bar that always swapped the two would pass the RTL cases alone.
    for (rtl, cmd, lit, unlit) in [
        (false, None, "Align Left", "Align Right"),
        (true, None, "Align Right", "Align Left"),
        (true, Some("para.alignLeft"), "Align Left", "Align Right"),
    ] {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("سلام دنیا")), Default::default());
        a.run("select.all", json!({})).unwrap();
        if rtl {
            a.run("para.rtl", json!({})).unwrap();
        }
        if let Some(cmd) = cmd {
            a.run(cmd, json!({})).unwrap();
        }
        // The bar writes in the interface fonts, which take effect the frame after they are set.
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(500.0, 200.0)).build_ui_state(
            move |ui, app: &mut WordApp| {
                if !fonts {
                    crate::theme::install_fonts(ui.ctx());
                    fonts = true;
                    return;
                }
                egui::Area::new(egui::Id::unique("mini_align_probe")).fixed_pos(pos2(10.0, 10.0)).show(ui.ctx(), |ui| {
                    let t = Tokens::get(ui.ctx());
                    content(app, ui, &t);
                });
            },
            a,
        );
        steps(&mut h);
        let checked = Tokens::get(&h.ctx).checked;
        let marked: Vec<Rect> = h
            .output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == checked => Some(r.rect),
                _ => None,
            })
            .collect();
        let is_marked = |label: &str| {
            let r = h.get_by_label(label).rect();
            marked.iter().any(|m| m.contains(r.center()))
        };
        assert!(is_marked(lit), "rtl={rtl} {cmd:?}: {lit} is marked");
        assert!(!is_marked(unlit), "rtl={rtl} {cmd:?}: {unlit} is not marked");
    }
}

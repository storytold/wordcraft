use egui::accesskit::Role;
use egui::{Vec2, pos2};
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;

type H = egui_kittest::Harness<'static, WordApp>;

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

/// The whole window over a document with `pick` selected, run until the bar is up.
fn bar_over(pick: &str) -> H {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "The quick brown fox jumps over the lazy dog"})).unwrap();
    a.run("select.text", json!({"text": pick})).unwrap();
    let mut h = crate::test_harness::builder().with_size(Vec2::new(1440.0, 900.0)).build_ui_state(
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

fn steps(h: &mut H) {
    crate::test_harness::frames(h, 6);
}

fn click_at(h: &mut H, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    steps(h);
}

fn hover_at(h: &mut H, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    steps(h);
}

fn press_key(h: &mut H, key: egui::Key, modifiers: egui::Modifiers) {
    h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    steps(h);
    // A real key goes up again: a key never released reads as held, and a second press of it as its repeat.
    h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    steps(h);
}

fn format_state(h: &mut H) -> Value {
    h.state_mut().session.run("format.state", &json!({})).unwrap()
}

/// The canvas viewport; the bar and the context menu's members are shown inside it, the ribbon is not.
fn canvas(h: &H) -> Rect {
    h.state().canvas.canvas_rect().expect("the document is shown")
}

/// The member named `label` inside the canvas, if one is shown there.
fn member(h: &H, label: &str) -> Option<Rect> {
    let c = canvas(h);
    h.query_all_by_label(label).map(|n| n.rect()).find(|r| c.contains_rect(*r))
}

/// Whether the selection bar's members are shown over the document.
fn bar_shown(h: &H) -> bool {
    member(h, "Bold").is_some()
}

/// The topmost node named `label`: the ribbon's, since it is above the document.
fn ribbon_member(h: &H, label: &str) -> egui::Pos2 {
    h.get_all_by_label(label).map(|n| n.rect().center()).min_by(|a, b| a.y.total_cmp(&b.y)).expect("the ribbon has the control")
}

fn size_box(h: &H) -> Rect {
    let c = canvas(h);
    h.query_all_by_role(Role::TextInput)
        .map(|n| n.rect())
        .filter(|r| c.contains_rect(*r))
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .expect("the bar has a size box")
}

fn font_box(h: &H) -> Rect {
    let c = canvas(h);
    h.query_all_by_role(Role::TextInput)
        .map(|n| n.rect())
        .filter(|r| c.contains_rect(*r))
        .min_by(|a, b| a.min.y.total_cmp(&b.min.y).then(a.min.x.total_cmp(&b.min.x)))
        .expect("the bar has a font box")
}

fn open_list_of(h: &mut H, boxed: Rect) {
    click_at(h, pos2(boxed.max.x + 8.0, boxed.center().y));
}

/// The arrow of the bar's split button named `label`: the member that opens a popup.
fn split_arrow(h: &H, label: &str) -> Rect {
    let c = canvas(h);
    let nodes: Vec<(Rect, bool)> = h
        .query_all_by_label_contains(label)
        .filter(|n| c.contains_rect(n.rect()))
        .map(|n| (n.rect(), n.accesskit_node().has_popup().is_some()))
        .collect();
    // The arrow is its own button, named "<label> options" (`colors::split_with`); the main part keeps the label.
    let arrow_name = format!("{label} options");
    let named = h.query_all_by_label(arrow_name.as_str()).map(|n| n.rect()).find(|r| c.contains_rect(*r));
    named
        .or_else(|| nodes.iter().find(|(_, popup)| *popup).map(|(r, _)| *r))
        .or_else(|| nodes.iter().max_by(|a, b| a.0.min.x.total_cmp(&b.0.min.x)).map(|(r, _)| *r))
        .expect("the bar has the split button")
}

fn secondary_click(h: &mut H, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        crate::test_harness::frame(h);
    }
    steps(h);
}

fn long_document(h: &mut H) {
    h.state_mut().run("caret.docStart", json!({})).unwrap();
    h.state_mut().run("text.insert", json!({"text": "x\n".repeat(150)})).unwrap();
    h.state_mut().run("select.text", json!({"text": "brown fox"})).unwrap();
    steps(h);
}

fn wheel_over_page(h: &mut H, dy: f32) {
    let page = canvas(h);
    h.event(egui::Event::PointerMoved(pos2(page.min.x + 20.0, page.center().y)));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy),
        phase: egui::TouchPhase::Start,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    });
    steps(h);
}

/// #535: the bar's members all fit: none overlaps another and all lie inside the document area.
#[test]
fn every_control_fits_inside_the_bar() {
    let h = bar_over("brown fox");
    let c = canvas(&h);
    let mut rects: Vec<(String, Rect)> = Vec::new();
    for label in ["Bold", "Italic", "Underline", "Align Left", "Center", "Align Right", "Justify", "Text Highlight Color", "Font Color"] {
        let r = member(&h, label).unwrap_or_else(|| panic!("the bar has {label}"));
        rects.push((label.to_string(), r));
    }
    rects.push(("font box".into(), font_box(&h)));
    rects.push(("size box".into(), size_box(&h)));
    // The framework's selection task bar is a group named by the label `mini_toolbar::show` gives it.
    let bar = h
        .query_all_by_role(Role::Group)
        .filter(|n| n.accesskit_node().label().as_deref() == Some("Formatting"))
        .map(|n| n.rect())
        .find(|r| c.contains_rect(*r))
        .expect("the selection bar lies inside the document area");
    for (name, r) in &rects {
        assert!(bar.contains_rect(*r), "{name} {r:?} must stay inside the bar {bar:?}");
    }
    for (i, (a_name, a)) in rects.iter().enumerate() {
        for (b_name, b) in &rects[i + 1..] {
            assert!(!a.shrink(0.5).intersects(b.shrink(0.5)), "{a_name} {a:?} overlaps {b_name} {b:?}");
        }
    }
}

/// #7447: the mini toolbar anchors to the start of the selection rather than its center,
/// so its position does not shift with the length of the selection.
#[test]
fn the_bar_anchors_to_the_selection_start_rather_than_its_center() {
    let h_short = bar_over("brown");
    let h_long = bar_over("brown fox jumps over the lazy dog");

    let bar_from = |h: &H| {
        let c = canvas(h);
        h.query_all_by_role(Role::Group)
            .filter(|n| n.accesskit_node().label().as_deref() == Some("Formatting"))
            .map(|n| n.rect())
            .find(|r| c.contains_rect(*r))
            .expect("the selection bar is shown inside the canvas")
    };

    let bar_short = bar_from(&h_short);
    let bar_long = bar_from(&h_long);

    assert!(
        (bar_short.min.x - bar_long.min.x).abs() < 1e-3,
        "the toolbar's horizontal position must anchor to the selection start and not shift with selection length: short x={}, long x={}",
        bar_short.min.x,
        bar_long.min.x,
    );

    // A selection starting at a later word on the same line tracks that word's start.
    let h_later = bar_over("jumps");
    let bar_later = bar_from(&h_later);
    assert!(
        bar_later.min.x > bar_short.min.x + 20.0,
        "a selection starting later on the line must place the toolbar further to the right: brown x={}, jumps x={}",
        bar_short.min.x,
        bar_later.min.x,
    );
}

/// #7375: the bar's formatting controls are stacked into two rows: Row 2 controls (Bold,
/// Align Left) sit strictly below Row 1 controls (Font, Font Size) rather than side-by-side.
#[test]
fn formatting_controls_stack_into_two_rows() {
    let h = bar_over("brown fox");
    let font = font_box(&h);
    let size = size_box(&h);
    let bold = member(&h, "Bold").expect("the bar has Bold");
    let align_left = member(&h, "Align Left").expect("the bar has Align Left");

    // Row 1 controls sit on the top row together.
    assert!((font.min.y - size.min.y).abs() < 1.0, "Font and Font Size must share Row 1: font {font:?}, size {size:?}");

    // Row 2 controls sit on the second row together.
    assert!((bold.min.y - align_left.min.y).abs() < 1.0, "Bold and Align Left must share Row 2: bold {bold:?}, align_left {align_left:?}");

    // Row 2 controls sit strictly below Row 1 controls.
    assert!(bold.min.y >= font.max.y, "Bold ({bold:?}) must sit strictly below Font ({font:?})");
    assert!(bold.min.y >= size.max.y, "Bold ({bold:?}) must sit strictly below Font Size ({size:?})");
    assert!(align_left.min.y >= font.max.y, "Align Left ({align_left:?}) must sit strictly below Font ({font:?})");
    assert!(align_left.min.y >= size.max.y, "Align Left ({align_left:?}) must sit strictly below Font Size ({size:?})");
}

/// Hovering any member of the bar previews its command on the selection; the pick is not made.
#[test]
fn hovering_a_bar_member_previews_its_command() {
    #[derive(Clone, Copy, Debug)]
    enum Member {
        Size,
        Font,
        Bold,
        HighlightButton,
        FontColorButton,
        HighlightSwatch,
    }
    for (member_kind, command) in [
        (Member::Size, "format.size"),
        (Member::Font, "format.font"),
        (Member::Bold, "format.bold"),
        (Member::HighlightButton, "format.highlight"),
        (Member::FontColorButton, "format.color"),
        (Member::HighlightSwatch, "format.highlight"),
    ] {
        let mut h = bar_over("brown fox");
        let (doc, sel, rev) = (h.state().session.doc.clone(), h.state().session.sel.clone(), h.state().session.rev());
        let size = format_state(&mut h).get("size").cloned();
        let font = format_state(&mut h).get("font").and_then(Value::as_str).unwrap_or_default().to_string();
        let target = match member_kind {
            Member::Size => {
                let b = size_box(&h);
                open_list_of(&mut h, b);
                h.get_by_label("20").rect().center()
            }
            Member::Font => {
                let b = font_box(&h);
                // The font picker's chevron is an adornment inside the field and a click on the name opens the list
                // too, so click the field itself (the size box's chevron lies beside it).
                click_at(&mut h, b.center());
                // The framework's font list is a tree of families (depth rows), so its rows are tree items.
                h.query_all_by_role(Role::TreeItem)
                    .filter(|n| {
                        (n.rect().min.y >= b.max.y || n.rect().max.y <= b.min.y) && n.accesskit_node().label().as_deref() != Some(font.as_str())
                    })
                    .filter(|n| n.accesskit_node().label().as_deref().is_some_and(|name| !name.ends_with('…')))
                    .map(|n| n.rect().center())
                    .next()
                    .expect("the list shows another family")
            }
            Member::Bold => member(&h, "Bold").expect("the bar has Bold").center(),
            Member::HighlightButton => member(&h, "Text Highlight Color").expect("the bar has the button").center(),
            Member::FontColorButton => member(&h, "Font Color").expect("the bar has the button").center(),
            Member::HighlightSwatch => {
                let arrow = split_arrow(&h, "Text Highlight Color");
                click_at(&mut h, arrow.center());
                h.query_all_by_label_contains("Yellow").next().expect("the list shows a swatch").rect().center()
            }
        };
        hover_at(&mut h, target);
        let previewed = h.state().canvas.command_previewed.clone();
        assert!(previewed.as_ref().is_some_and(|(id, _)| id == command), "{member_kind:?} previews {command}, got {previewed:?}");
        assert_ne!(crate::canvas::page_doc(h.state()), &doc, "{member_kind:?}: the page shows the preview");
        assert_eq!((&h.state().session.doc, &h.state().session.sel, h.state().session.rev()), (&doc, &sel, rev), "{member_kind:?}");
        assert_eq!(format_state(&mut h).get("size").cloned(), size);

        hover_at(&mut h, pos2(5.0, 5.0));
        assert_eq!(h.state().canvas.command_previewed, None, "{member_kind:?}: moving off the bar clears the preview");
    }
}

/// An alignment preview moves the previewed selection; the bar stays anchored to the live
/// selection so the hovered button remains under the pointer.
#[test]
fn the_bar_stays_put_under_an_alignment_preview() {
    let mut h = bar_over("brown fox");
    let anchor = h.state().canvas.mini_anchor.unwrap();
    let align_right = member(&h, "Align Right").expect("the bar has Align Right");
    hover_at(&mut h, align_right.center());
    assert_eq!(h.state().canvas.command_previewed, Some(("para.alignRight".to_string(), json!({}))));
    assert_eq!(h.state().canvas.mini_anchor, Some(anchor), "the alignment preview must not move the toolbar");
}

/// While a preview is shown the page holds its scroll position, though the previewed text
/// reflows and the caret would be scrolled into view.
#[test]
fn the_page_holds_its_place_while_a_preview_is_shown() {
    let mut h = bar_over("brown fox");
    long_document(&mut h);
    wheel_over_page(&mut h, -60.0);
    let held = h.state().canvas.view.offset;
    let bold = member(&h, "Bold").expect("the bar follows the selection");
    hover_at(&mut h, bold.center());
    assert!(h.state().canvas.command_previewed.is_some(), "hovering Bold starts a preview");
    for _ in 0..4 {
        h.state_mut().canvas.scroll_to_caret = true;
        crate::test_harness::frame(&mut h);
    }
    assert_eq!(h.state().canvas.view.offset, held, "the preview does not move the page");
    hover_at(&mut h, pos2(5.0, 5.0));
    assert_eq!(h.state().canvas.preview_scroll, None, "the hold ends with the preview");
}

/// With no preview shown the bar's anchor follows the selection as the page scrolls.
#[test]
fn the_bar_anchor_follows_the_selection_as_the_page_scrolls() {
    let mut h = bar_over("brown fox");
    long_document(&mut h);
    let anchored = h.state().canvas.mini_anchor.unwrap();
    wheel_over_page(&mut h, 120.0);
    assert_ne!(h.state().canvas.mini_anchor, Some(anchored), "the toolbar anchor follows the live selection as the viewport scrolls");
}

/// #2959: formatting is available alongside the text's right-click menu, using the selection that
/// was clicked. Its lists stay usable inside the context menu.
#[test]
fn context_menu_toolbar_formats_the_selection_and_opens_its_size_list() {
    let mut h = bar_over("brown fox");
    let selection = h.state().session.sel.clone();
    let caret = h.state_mut().session.layout().caret(&wordcraft_doc::Pos::body(0, 12)).unwrap();
    let at = crate::canvas::page_to_screen(h.state_mut(), caret.page, caret.x, caret.top + caret.height / 2.0).unwrap();
    secondary_click(&mut h, at);
    assert!(h.state().canvas.context_menu_open);
    assert_eq!(h.state().session.sel, selection, "right-click inside the selection keeps it");
    let bold = member(&h, "Bold").expect("the context menu has Bold");
    let doc = h.state().session.doc.clone();
    hover_at(&mut h, bold.center());
    assert_eq!(h.state().canvas.command_previewed, None, "the context toolbar starts no preview");
    assert_eq!(&h.state().session.doc, &doc, "hovering a context-menu control does not preview a document change");
    click_at(&mut h, bold.center());
    assert_eq!(format_state(&mut h).get("bold"), Some(&json!(true)));
    assert_eq!(h.state().session.sel, selection);
    assert!(h.state().canvas.context_menu_open, "formatting keeps the context toolbar available");

    let b = size_box(&h);
    open_list_of(&mut h, b);
    let twenty = h.get_by_label("20").rect().center();
    click_at(&mut h, twenty);
    assert_eq!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(20.0));
    assert_eq!(h.state().session.sel, selection);
    assert!(h.state().canvas.context_menu_open, "picking a font size keeps the parent menu available");

    let top = canvas(&h).top();
    let copy = h.query_all_by_label_contains("Copy").map(|n| n.rect()).find(|r| r.min.y > top).expect("the menu has Copy");
    click_at(&mut h, copy.center());
    assert!(!h.state().canvas.context_menu_open, "ordinary menu commands still close the menu");
}

#[test]
fn context_menu_toolbar_opens_at_a_caret_from_the_keyboard_and_dismisses() {
    let mut h = bar_over("brown fox");
    h.state_mut().run("caret.set", json!({"pos": wordcraft_doc::Pos::body(0, 4)})).unwrap();
    steps(&mut h);
    let caret = h.state().session.sel.clone();
    assert!(caret.is_collapsed());
    press_key(&mut h, egui::Key::F10, egui::Modifiers::SHIFT);
    assert!(h.state().canvas.context_menu_open);
    let bold = member(&h, "Bold").expect("the keyboard menu has Bold at a caret too");
    click_at(&mut h, bold.center());
    assert_eq!(format_state(&mut h).get("bold"), Some(&json!(true)));
    assert_eq!(h.state().session.sel, caret);
    // The framework's Escape closes the top layer first: the bar (where the click on Bold left the keyboard), then the
    // menu under it.
    // Each Escape closes one layer (the bar's own popups, the bar, the menu): press until the menu is gone.
    for _ in 0..4 {
        press_key(&mut h, egui::Key::Escape, Default::default());
        if !h.state().canvas.context_menu_open {
            break;
        }
    }
    assert!(!h.state().canvas.context_menu_open, "Escape dismisses the keyboard-opened menu");
    h.event(egui::Event::Text(" typed".into()));
    steps(&mut h);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains(" typed"), "typing resumes in the document");

    h.state_mut().run("ui.contextMenu", json!({})).unwrap();
    steps(&mut h);
    assert!(h.state().canvas.context_menu_open, "ui.contextMenu opens it again");
    assert!(h.state().canvas.context_request.is_none(), "the pass consumed the ask");
    let c = canvas(&h);
    click_at(&mut h, c.right_bottom() - Vec2::splat(10.0));
    assert!(!h.state().canvas.context_menu_open, "an outside click dismisses both panels");
}

#[test]
fn context_menu_toolbar_respects_read_mode_and_the_toolbar_preference() {
    for read_mode in [false, true] {
        let mut h = bar_over("brown fox");
        h.state_mut().session.view.read_mode = read_mode;
        h.state_mut().ui.mini_toolbar = read_mode;
        h.state_mut().run("ui.contextMenu", json!({})).unwrap();
        steps(&mut h);
        assert!(h.state().canvas.context_menu_open, "the text menu remains available");
        let top = canvas(&h).top();
        assert!(member(&h, "Bold").is_none(), "the context toolbar is suppressed, read_mode={read_mode}");
        assert!(h.query_all_by_label_contains("Copy").any(|n| n.rect().min.y > top), "the menu still offers Copy");
    }
}

#[test]
fn escape_in_a_ribbon_list_does_not_open_the_context_menu() {
    let mut h = bar_over("brown fox");
    let current = format_state(&mut h).get("size").and_then(Value::as_f64).unwrap_or_default();
    let current = if current.fract() == 0.0 { format!("{current:.0}") } else { current.to_string() };
    let size = h
        .get_all_by_role(Role::TextInput)
        .filter(|n| n.value().as_deref() == Some(current.as_str()))
        .map(|n| n.rect())
        .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .expect("the ribbon has a size field");
    click_at(&mut h, pos2(size.max.x + 8.0, size.center().y));
    let list_open = |h: &H| h.query_all_by_label("28").next().is_some();
    assert!(list_open(&h), "the size list is open");
    press_key(&mut h, egui::Key::Escape, Default::default());
    assert!(!list_open(&h), "Escape closes the size list");
    assert!(!h.state().canvas.context_menu_open, "dismissing another popup must not open the text context menu");
}

/// Turning to the ribbon puts the bar away (it covered the ribbon's drop-downs); it comes back
/// for the next selection, even one over the same text.
#[test]
fn a_click_on_the_ribbon_puts_the_bar_away_until_the_selection_changes() {
    let mut h = bar_over("brown fox");
    assert!(visible(h.state()) && bar_shown(&h), "the bar is up over the selection");
    let at = ribbon_member(&h, "Bold");
    click_at(&mut h, at);
    assert_eq!(format_state(&mut h).get("bold"), Some(&json!(true)), "the click ran the ribbon's Bold");
    assert!(!visible(h.state()), "a click on the ribbon puts the bar away");
    assert!(!bar_shown(&h), "the members are no longer drawn");

    h.state_mut().run("select.text", json!({"text": "lazy"})).unwrap();
    steps(&mut h);
    assert!(visible(h.state()) && bar_shown(&h), "a new selection brings the bar back");

    let at = ribbon_member(&h, "Bold");
    click_at(&mut h, at);
    assert!(!bar_shown(&h));
    // A press in the document selects the same text again: the bar comes back.
    let (from, to) = {
        let layout = h.state_mut().session.layout();
        let a = layout.caret(&wordcraft_doc::Pos::body(0, 35)).unwrap();
        let b = layout.caret(&wordcraft_doc::Pos::body(0, 39)).unwrap();
        (
            crate::canvas::page_to_screen(h.state_mut(), a.page, a.x, a.top + a.height / 2.0).unwrap(),
            crate::canvas::page_to_screen(h.state_mut(), b.page, b.x, b.top + b.height / 2.0).unwrap(),
        )
    };
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    crate::test_harness::frame(&mut h);
    h.event(egui::Event::PointerMoved(to));
    crate::test_harness::frame(&mut h);
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    steps(&mut h);
    assert!(bar_shown(&h), "a press in the document brings the bar back for the same text");
}

/// The ribbon's size list hangs over the page: picking from it is no press in the document, so
/// the bar stays away.
#[test]
fn picking_from_a_ribbon_list_over_the_page_keeps_the_bar_away() {
    let mut h = bar_over("brown fox");
    let size = format_state(&mut h).get("size").and_then(Value::as_f64).unwrap_or_default();
    let size = if size.fract() == 0.0 { format!("{size:.0}") } else { size.to_string() };
    let size_box = h
        .get_all_by_role(Role::TextInput)
        .filter(|n| n.value().as_deref() == Some(size.as_str()))
        .map(|n| n.rect())
        .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .expect("the ribbon has a size box");
    click_at(&mut h, pos2(size_box.max.x + 8.0, size_box.center().y));
    assert!(!bar_shown(&h), "opening the ribbon's list puts the bar away");
    let item = h.get_by_label("28").rect().center();
    assert!(canvas(&h).contains(item), "the list's 28 should hang over the page for this test");
    click_at(&mut h, item);
    assert_eq!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(28.0), "the ribbon's size list applied 28 pt");
    assert!(!visible(h.state()) && !bar_shown(&h), "the bar stays away after a pick from the ribbon's list");
}

/// Picking from one of the bar's own lists keeps the bar up: a press in its popup is not "outside".
#[test]
fn picking_from_the_bars_own_list_keeps_it() {
    let mut h = bar_over("brown fox");
    let b = size_box(&h);
    open_list_of(&mut h, b);
    let twenty = h.get_by_label("20").rect().center();
    click_at(&mut h, twenty);
    assert_eq!(format_state(&mut h).get("size").and_then(Value::as_f64), Some(20.0), "the bar's size list applied 20 pt");
    assert!(visible(h.state()) && bar_shown(&h), "the bar stays up after its own list is used");
}

/// #2105: No Color becomes the Highlight button's choice: the next click removes highlighting.
#[test]
fn no_color_becomes_the_highlight_buttons_choice() {
    let mut h = bar_over("brown fox");
    h.state_mut().run("format.highlight", json!({"color": "yellow"})).unwrap();
    steps(&mut h);
    let arrow = split_arrow(&h, "Text Highlight Color");
    click_at(&mut h, arrow.center());
    let no_color = h.get_by_label_contains("No Color").rect().center();
    click_at(&mut h, no_color);
    assert_eq!(format_state(&mut h).get("highlight"), Some(&Value::Null), "No Color removed the highlight");
    assert_eq!(h.state().canvas.last_highlight, "none");

    // Other highlighted text, then the button itself (left of its arrow): no highlight.
    h.state_mut().run("select.text", json!({"text": "lazy"})).unwrap();
    h.state_mut().run("format.highlight", json!({"color": "yellow"})).unwrap();
    steps(&mut h);
    let arrow = split_arrow(&h, "Text Highlight Color");
    click_at(&mut h, pos2(arrow.min.x - 12.0, arrow.center().y));
    assert_eq!(format_state(&mut h).get("highlight"), Some(&Value::Null), "the button applies No Color, not yellow again");

    // A stale or corrupted remembered value looks like No Color and must act like it too.
    h.state_mut().canvas.last_highlight = "unknown".into();
    h.state_mut().run("format.highlight", json!({"color": "yellow"})).unwrap();
    steps(&mut h);
    let arrow = split_arrow(&h, "Text Highlight Color");
    click_at(&mut h, pos2(arrow.min.x - 12.0, arrow.center().y));
    assert_eq!(format_state(&mut h).get("highlight"), Some(&Value::Null), "an unknown remembered colour is normalized to No Color");
}

/// #3039: picking Automatic from the bar's font-colour list hands the keyboard back to the document.
#[test]
fn the_font_colour_menus_automatic_returns_focus_to_the_document() {
    let mut h = bar_over("brown fox");
    h.state_mut().run("format.color", json!({"color": "FF0000"})).unwrap();
    steps(&mut h);
    let arrow = split_arrow(&h, "Font Color");
    click_at(&mut h, arrow.center());
    assert!(!h.state().canvas.focused(&h.ctx), "the open list holds the keyboard");
    let automatic = h.get_by_label_contains("Automatic").rect().center();
    click_at(&mut h, automatic);
    assert!(h.state().canvas.focused(&h.ctx), "Automatic returns focus to the document");
    assert_ne!(format_state(&mut h).get("color"), Some(&json!("FF0000")), "the pick ran the font colour command");
}

/// #3038: a pick from the bar's highlight grid hands the keyboard back to the document.
#[test]
fn picking_a_highlight_swatch_returns_focus_to_the_document() {
    let mut h = bar_over("brown fox");
    let arrow = split_arrow(&h, "Text Highlight Color");
    click_at(&mut h, arrow.center());
    assert!(!h.state().canvas.focused(&h.ctx), "the open list holds the keyboard");
    let first = h.query_all_by_label_contains("Yellow").next().expect("the list shows a swatch").rect().center();
    click_at(&mut h, first);
    assert!(h.state().canvas.focused(&h.ctx), "the swatch pick returns focus to the document");
    assert!(format_state(&mut h).get("highlight").is_some_and(|v| !v.is_null()), "the swatch applied a highlight");
}

/// #3579: the bar marks the alignment as seen on the page, the one its buttons set, as the ribbon does.
#[test]
fn the_alignment_buttons_mark_the_paragraph_as_seen_on_the_page() {
    // A left-to-right paragraph first: a bar that always swapped the two would pass the RTL cases alone.
    let cases: [(&str, &[(&str, Value)], &str, &str); 4] = [
        ("سلام دنیا", &[("select.all", json!({}))], "Align Left", "Align Right"),
        ("سلام دنیا", &[("select.all", json!({})), ("para.rtl", json!({}))], "Align Right", "Align Left"),
        ("سلام دنیا", &[("select.all", json!({})), ("para.rtl", json!({})), ("para.alignLeft", json!({}))], "Align Left", "Align Right"),
        (
            "One\nTwo",
            &[("select.text", json!({"text": "One"})), ("para.alignRight", json!({})), ("select.paragraph", json!({}))],
            "Align Right",
            "Align Left",
        ),
    ];
    for (text, commands, lit, unlit) in cases {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(text)), Default::default());
        for (cmd, params) in commands {
            a.run(cmd, params.clone()).unwrap();
        }
        // The bar writes in the interface fonts, which take effect the frame after they are set.
        let mut fonts = false;
        let mut h = crate::test_harness::builder().with_size(Vec2::new(500.0, 200.0)).build_ui_state(
            move |ui, app: &mut WordApp| {
                if !fonts {
                    // The runtime and its fonts are installed by the harness; the bar waits a frame.
                    fonts = true;
                    return;
                }
                egui::Area::new(egui::Id::unique("mini_align_probe")).fixed_pos(pos2(10.0, 10.0)).show(ui.ctx(), |ui| {
                    members(app, ui, true);
                });
            },
            a,
        );
        steps(&mut h);
        let toggled = |label: &str| h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True);
        assert!(toggled(lit), "{text:?} {commands:?}: {lit} is marked");
        assert!(!toggled(unlit), "{text:?} {commands:?}: {unlit} is not marked");
    }
}

/// After a click on a member no preview shows until the pointer leaves it: a toggle would
/// otherwise preview its other state under the pointer and look as if it flashed.
#[test]
fn a_clicked_member_holds_its_preview_until_the_pointer_leaves() {
    for (label, command) in [("Increase Font Size", None), ("Bold", Some("format.bold"))] {
        let mut h = bar_over("brown fox");
        let r = member(&h, label).unwrap_or_else(|| panic!("the bar has {label}"));
        let at = r.center();
        hover_at(&mut h, at);
        assert!(h.state().canvas.command_previewed.is_some(), "{label}: hovering previews");
        click_at(&mut h, at);
        assert_eq!(h.state().canvas.command_previewed, None, "{label}: no preview after the click while the pointer stays");
        hover_at(&mut h, pos2(5.0, 5.0));
        hover_at(&mut h, at);
        let previewed = h.state().canvas.command_previewed.clone();
        assert!(previewed.is_some(), "{label}: coming back previews again");
        if let Some(command) = command {
            assert_eq!(previewed.map(|(id, _)| id).as_deref(), Some(command));
            assert_ne!(crate::canvas::page_doc(h.state()), &h.state().session.doc, "{label}: the page shows the toggled-back state");
        }
    }
}

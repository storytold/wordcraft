use super::{Content, Pane};
use craft_ui::panes::PaneContent;
use egui::vec2;

/// #1902: Clipboard Delete returns focus to the document like paste does.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn clipboard_delete_returns_focus_to_the_document() {
    use egui_kittest::kittest::Queryable;

    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
    app.run("select.all", serde_json::json!({})).unwrap();
    app.run("edit.copy", serde_json::json!({})).unwrap();
    assert_eq!(app.session.clip_history.items().len(), 1);
    app.run("edit.clipboardPane", serde_json::json!({"value": true})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut crate::WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    // Focus the button for real: a bare accesskit click leaves egui focus empty, and an
    // unrelated chrome-click helper would hand focus to the canvas on its own.
    h.get_by_label("Delete").focus();
    h.run_steps(2);
    h.state_mut().canvas.want_focus = false;
    h.get_by_label("Delete").click();
    h.run_steps(4);
    assert!(h.state().canvas.focused, "Delete returns focus to the document");
    assert!(h.state().session.clip_history.items().is_empty(), "the item was deleted");
}

/// F6 past the last pane gives the document the keyboard, and typing reaches it (#1756).
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn f6_past_the_last_pane_types_into_the_document() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    app.run("view.navigationPane", serde_json::json!({"value": true})).unwrap();
    app.run("edit.clipboardPane", serde_json::json!({"value": true})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut crate::WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    assert!(h.state().canvas.focused);
    let f6 = egui::Event::Key { key: egui::Key::F6, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    for pane in ["Navigation", "Clipboard"] {
        h.input_mut().events.push(f6.clone());
        h.run_steps(2);
        assert!(!h.state().canvas.focused, "F6 moved the keyboard to the {pane} pane");
    }
    h.input_mut().events.push(f6);
    h.run_steps(2);
    assert!(h.state().canvas.focused, "F6 past the last pane returns to the document");
    h.input_mut().events.push(egui::Event::Text("typed".into()));
    h.run_steps(2);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "typed");
}

fn pane_harness(app: crate::WordApp) -> egui_kittest::Harness<'static, crate::WordApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut crate::WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn press(h: &mut egui_kittest::Harness<'static, crate::WordApp>, key: egui::Key, modifiers: egui::Modifiers) {
    h.input_mut().events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.run_steps(2);
}

/// F6 and Shift+F6 with a dialog open leave the keyboard in the dialog: no pane title bar and
/// no document behind it takes it.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn f6_in_an_open_dialog_keeps_focus_there() {
    for panes in [true, false] {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
        app.run("view.navigationPane", serde_json::json!({"value": panes})).unwrap();
        app.run("ui.dialog", serde_json::json!({"name": "replace"})).unwrap();
        let mut h = pane_harness(app);
        let focused = h.ctx.memory(|m| m.focused());
        for shift in [false, true] {
            press(&mut h, egui::Key::F6, if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE });
            assert_eq!(h.ctx.memory(|m| m.focused()), focused, "panes open: {panes}, shift: {shift}");
            assert!(!h.state().canvas.focused && h.state().dialog.is_some(), "panes open: {panes}, shift: {shift}");
        }
    }
}

/// Ctrl and F-key shortcuts run while F6 has put the keyboard on a pane's title bar; a pane's
/// text field keeps Ctrl+H for itself.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn shortcuts_run_while_a_pane_title_bar_has_focus() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    app.run("view.navigationPane", serde_json::json!({"value": true})).unwrap();
    let mut h = pane_harness(app);
    press(&mut h, egui::Key::F6, egui::Modifiers::NONE);
    assert!(!h.state().canvas.focused && h.ctx.memory(|m| m.focused()).is_some(), "the title bar has the keyboard");
    press(&mut h, egui::Key::H, egui::Modifiers::COMMAND);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("replace"), "Ctrl+H opened Replace");
    press(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(h.state().dialog.is_none());

    h.state_mut().nav_search_focus = true;
    h.run_steps(2);
    assert!(h.ctx.text_edit_focused());
    press(&mut h, egui::Key::H, egui::Modifiers::COMMAND);
    assert!(h.state().dialog.is_none(), "the search field keeps Ctrl+H");
}

/// A dialog button with the keyboard keeps Ctrl+Z away from the document behind the dialog.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn shortcuts_stay_out_of_the_document_while_a_dialog_button_has_focus() {
    use egui_kittest::kittest::Queryable;

    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    app.run("text.insert", serde_json::json!({"text": "kept"})).unwrap();
    app.run("ui.dialog", serde_json::json!({"name": "wordCount"})).unwrap();
    let mut h = pane_harness(app);
    // Two buttons are labelled Close (the dialog's title bar and its own row); either will do.
    if let Some(close) = h.query_all_by_label("Close").next() {
        close.focus();
    }
    h.run_steps(2);
    assert!(!h.state().canvas.focused, "the document gave up the keyboard");
    assert!(!h.ctx.text_edit_focused() && h.ctx.memory(|m| m.focused()).is_some(), "Close has the keyboard");
    press(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "kept", "Ctrl+Z left the document alone");
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("wordCount"));
}

/// Escape during a pane's title-bar drag cancels the drag and leaves full screen on.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn escape_during_a_pane_drag_keeps_full_screen() {
    use egui_kittest::kittest::Queryable;

    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    app.run("view.navigationPane", serde_json::json!({"value": true})).unwrap();
    app.run("view.fullScreen", serde_json::json!({"value": true})).unwrap();
    let mut h = pane_harness(app);
    let title = h.get_by_label_contains("Move Pane: ").rect().center();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.input_mut().events.extend([egui::Event::PointerMoved(title), button(title, true)]);
    h.run_steps(2);
    let away = title + egui::vec2(300.0, 200.0);
    h.input_mut().events.push(egui::Event::PointerMoved(away));
    h.run_steps(2);
    assert!(h.ctx.dragged_id().is_some(), "the title bar is dragged");
    press(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(h.state().session.view.full_screen);
    h.input_mut().events.push(button(away, false));
    h.run_steps(2);
    assert!(h.state().session.view.full_screen);
    let docked = craft_ui::panes::Place::Docked(craft_ui::panes::Edge::Start);
    assert_eq!(super::pane_state(h.state(), Pane::Navigation).place, docked, "the drag was cancelled");
}

/// With Navigation docked left, Comments dropped over Navigation's outer third docks left of
/// it, where the preview showed, and the order survives a restart; over Navigation's inner
/// third it goes back inside.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn comments_dropped_over_navigations_outer_third_docks_left_of_it() {
    use egui_kittest::kittest::Queryable;

    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    app.run("view.navigationPane", serde_json::json!({"value": true})).unwrap();
    app.run("view.commentsPane", serde_json::json!({"value": true})).unwrap();
    let mut h = pane_harness(app);
    // A docked pane's title bar starts at its panel's left edge.
    let panel = |h: &egui_kittest::Harness<'static, crate::WordApp>, pane: Pane| h.get_by_label(&format!("Move Pane: {}", pane.title())).rect();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let drag = |h: &mut egui_kittest::Harness<'static, crate::WordApp>, to: egui::Pos2| {
        let from = h.get_by_label("Move Pane: Comments").rect().center();
        h.input_mut().events.extend([egui::Event::PointerMoved(from), button(from, true)]);
        h.run_steps(1);
        h.input_mut().events.push(egui::Event::PointerMoved(from + egui::vec2(-8.0, 6.0)));
        h.run_steps(1);
        h.input_mut().events.push(egui::Event::PointerMoved(to));
        h.run_steps(1);
        h.input_mut().events.push(button(to, false));
        h.run_steps(3);
    };
    let nav = panel(&h, Pane::Navigation);
    drag(&mut h, egui::pos2(nav.left() + 60.0, nav.center().y));
    let (comments, nav_after) = (panel(&h, Pane::Comments), panel(&h, Pane::Navigation));
    assert_eq!(comments.left(), nav.left(), "Comments is outermost");
    assert!(nav_after.left() > comments.right(), "Navigation sits inside it");
    assert_eq!(super::open(h.state()), vec![Pane::Comments, Pane::Navigation]);

    let saved = serde_json::to_vec(&h.state().prefs()).unwrap();
    let mut next = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    next.apply_prefs(serde_json::from_slice(&saved).unwrap());
    next.session.view.comments_pane = true;
    assert_eq!(super::open(&next), vec![Pane::Comments, Pane::Navigation], "the order is kept");

    let nav = panel(&h, Pane::Navigation);
    let width = Pane::Navigation.default_state().size.unwrap();
    drag(&mut h, egui::pos2(nav.left() + width * 0.85, nav.center().y));
    assert_eq!(super::open(h.state()), vec![Pane::Navigation, Pane::Comments]);
    assert!(panel(&h, Pane::Comments).left() > panel(&h, Pane::Navigation).right(), "back inside Navigation");

    let mut junk = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    junk.apply_prefs(serde_json::from_str(r#"{"paneOrder": ["comments", 3, "nope", "comments", "navigation"], "navPane": true}"#).unwrap());
    assert_eq!(junk.ui.pane_order, vec!["comments", "navigation"], "unknown and repeated keys are dropped");
    for bad in [serde_json::json!({"pane": "comments", "before": "comments"}), serde_json::json!({"pane": "comments", "before": 7})] {
        assert!(junk.run("ui.paneDock", bad.clone()).is_err(), "{bad}");
    }
    junk.run("ui.paneDock", serde_json::json!({"pane": "navigation", "before": null})).unwrap();
    assert_eq!(junk.ui.pane_order.last().map(String::as_str), Some("navigation"));
}

/// An F6 the user bound runs its command; plain F6 still moves to the panes when only
/// Shift+F6 is bound.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_user_bound_f6_runs_its_command() {
    for key in ["F6", "Shift+F6"] {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
        app.run("view.navigationPane", serde_json::json!({"value": true})).unwrap();
        app.session.keymap = wordcraft_engine::keymap::KeyMap::from_value(&serde_json::json!({"assigned": {key: "view.focus"}}));
        let mut h = pane_harness(app);
        assert!(h.state().canvas.focused);
        let m = if key == "F6" { egui::Modifiers::NONE } else { egui::Modifiers::SHIFT };
        press(&mut h, egui::Key::F6, m);
        assert!(h.state().session.view.focus_mode, "{key} ran its command");
        press(&mut h, egui::Key::F6, m);
        assert!(!h.state().session.view.focus_mode, "{key} ran its command again");
        if key == "Shift+F6" {
            h.state_mut().canvas.want_focus = true;
            h.run_steps(2);
            press(&mut h, egui::Key::F6, egui::Modifiers::NONE);
            assert!(!h.state().canvas.focused, "plain F6 moved the keyboard to the pane");
        }
    }
}

/// Every pane's body fits a pane at the 200 pt minimum width (the host clips what doesn't):
/// button rows wrap and fields narrow. Restrict Editing shows its style list and password
/// fields.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn pane_bodies_fit_the_narrowest_pane() {
    // 200 pt less the 10 pt body margins and the frame's 1 pt strokes.
    const BODY: f32 = 178.0;
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
    app.run("select.all", serde_json::json!({})).unwrap();
    app.run("edit.copy", serde_json::json!({})).unwrap();
    app.restrict =
        crate::dialogs_review::RestrictPane { open: true, formatting: true, show_styles: true, editing: true, asking: true, ..Default::default() };
    let panes = [
        Pane::Navigation,
        Pane::Clipboard,
        Pane::Styles,
        Pane::StyleInspector,
        Pane::Accessibility,
        Pane::AltText,
        Pane::FormatShape,
        Pane::RestrictEditing,
        Pane::Comments,
    ];
    let widths = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = widths.clone();
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 1200.0)).build_ui_state(
        move |ui, app: &mut crate::WordApp| {
            // Fonts take effect the frame after they are installed.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            let mut row = Vec::new();
            for pane in panes {
                let width = ui
                    .allocate_ui_with_layout(vec2(BODY, 1200.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_max_width(BODY);
                        Content { app: &mut *app }.body(ui, &pane);
                        ui.min_rect().width()
                    })
                    .inner;
                row.push((pane, width));
            }
            *seen.borrow_mut() = row;
        },
        app,
    );
    h.run_steps(3);
    let widths = widths.borrow();
    assert_eq!(widths.len(), panes.len());
    let wide: Vec<_> = widths.iter().filter(|(_, width)| *width > BODY + 0.5).collect();
    assert!(wide.is_empty(), "wider than the pane: {wide:?}");
}

/// A pane other than Styles floated from its menu comes back floating, at the same rect, after
/// a restart; junk in `panePlaces` reads as the defaults (#1756).
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_floated_pane_floats_again_after_a_restart() {
    use craft_ui::panes::Place;
    use egui_kittest::kittest::Queryable;
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    app.run("view.navigationPane", serde_json::json!({"value": true})).unwrap();
    let mut h = pane_harness(app);
    let bar = h.get_by_label_contains("Move Pane: ").rect();
    let on_row = |r: egui::Rect| r.center().y > bar.top() && r.center().y < bar.bottom();
    h.get_all_by_label("Pane Options").find(|n| on_row(n.rect())).expect("the menu button beside the title bar").click();
    h.run_steps(2);
    h.get_by_label("Float").click();
    h.run_steps(3);
    assert_eq!(super::pane_state(h.state(), Pane::Navigation).place, Place::Floating);
    let rect = h.state().ui.pane_places.get("navigation").and_then(|p| p.float).expect("the float rect is kept");

    let saved = serde_json::to_vec(&h.state().prefs()).unwrap();
    let mut next = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    next.apply_prefs(serde_json::from_slice(&saved).unwrap());
    let state = super::pane_state(&next, Pane::Navigation);
    assert_eq!((state.place, state.float_rect), (Place::Floating, Some(rect)));
    assert_eq!(super::pane_state(&next, Pane::Clipboard), Pane::Clipboard.default_state(), "other panes keep their defaults");

    let mut junk = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    junk.apply_prefs(
        serde_json::from_str(
            r#"{"panePlaces": {"navigation": {"dock": "top", "float": [1, "x", 3, 4]}, "clipboard": 7, "nope": {}}, "navPane": true}"#,
        )
        .unwrap(),
    );
    assert_eq!(super::pane_state(&junk, Pane::Navigation), Pane::Navigation.default_state(), "junk reads as the default");
    assert!(junk.session.view.nav_pane, "the rest of the file still loads");
    let mut odd = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    odd.apply_prefs(serde_json::from_str(r#"{"panePlaces": [1, 2], "navPane": true}"#).unwrap());
    assert!(odd.ui.pane_places.is_empty() && odd.session.view.nav_pane);
}

/// `ui.paneDock` docks or floats any pane and reads its place, as the pane's menu does
/// (#1756); Styles goes through `view.stylesPaneDock`; junk is refused and changes nothing.
#[test]
fn pane_dock_command_docks_floats_and_reads_each_pane() {
    use craft_ui::panes::{Edge, Place};
    use serde_json::json;
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    assert_eq!(app.run("ui.paneDock", json!({"pane": "navigation"})).unwrap(), json!({"pane": "navigation", "dock": "left"}));
    assert_eq!(app.run("ui.paneDock", json!({"pane": "comments"})).unwrap(), json!({"pane": "comments", "dock": "right"}));
    assert_eq!(app.run("ui.paneDock", json!({"pane": "navigation", "dock": "float"})).unwrap()["dock"], "float");
    assert_eq!(super::pane_state(&app, Pane::Navigation).place, Place::Floating);
    app.run("ui.paneDock", json!({"pane": "comments", "dock": "left"})).unwrap();
    assert_eq!(super::pane_state(&app, Pane::Comments).place, Place::Docked(Edge::Start));
    assert_eq!(app.run("ui.paneDock", json!({"pane": "styles", "dock": "float"})).unwrap(), json!({"pane": "styles", "dock": "float"}));
    assert_eq!(app.session.view.styles_pane_dock, wordcraft_engine::StylesPaneDock::Floating);
    assert!(!app.ui.pane_places.contains_key("styles"), "Styles keeps its place in the view state");
    for junk in [
        json!({}),
        json!({"pane": "nope", "dock": "left"}),
        json!({"pane": 3}),
        json!({"pane": "navigation", "dock": "top"}),
        json!({"pane": "navigation", "dock": 1}),
        json!({"pane": "navigation", "dock": null}),
    ] {
        assert!(app.run("ui.paneDock", junk.clone()).is_err(), "{junk}");
    }
    assert_eq!(super::pane_state(&app, Pane::Navigation).place, Place::Floating, "junk changed nothing");
}

/// #3032: Delete is greyed out for built-in styles in the Styles pane (the engine refuses
/// them — `crates/engine/src/cmd/para.rs`), while custom styles keep it enabled.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn built_in_styles_grey_out_delete_in_the_styles_pane() {
    use egui_kittest::kittest::{NodeT, Queryable};

    /// Draw only the pane, so each style name labels exactly one node — the ribbon's Styles
    /// gallery shows the same names — right-click the row, and report whether its Delete is
    /// greyed out.
    fn delete_is_disabled(style: &str) -> bool {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
        app.run("styles.create", serde_json::json!({"name": "Callout"})).unwrap();
        // The pane writes in the "semibold" family, which the app installs with the rest of the
        // interface fonts (`chrome::title_bar`). Fonts take effect the frame after they are set,
        // so the first frame only installs them and the pane is drawn from the second on.
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 1200.0)).build_ui_state(
            |ui, app: &mut crate::WordApp| {
                if !fonts {
                    crate::theme::install_fonts(ui.ctx());
                    fonts = true;
                    return;
                }
                super::styles(app, ui);
            },
            app,
        );
        h.run_steps(4);
        h.get_by_label(style).click_secondary();
        h.run_steps(3);
        h.get_by_label("Delete").accesskit_node().is_disabled()
    }
    assert!(delete_is_disabled("Normal"), "the built-in style's Delete is greyed out");
    assert!(!delete_is_disabled("Callout"), "the custom style's Delete stays enabled");
}

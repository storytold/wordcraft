use super::Pane;
use super::nav::nav_headings;
use craft::ui::patterns::dock::{PanePlace, PaneRequest};
use craft::ui::runtime::env::PaneEdge;
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
    let mut h = pane_harness(app);
    // Focus the button for real: a bare accesskit click leaves egui focus empty, and an
    // unrelated chrome-click helper would hand focus to the canvas on its own.
    h.get_by_label("Delete").focus();
    h.run_steps(2);
    assert!(!h.state().canvas.focused(&h.ctx), "the button has the keyboard");
    h.get_by_label("Delete").click();
    h.run_steps(4);
    assert!(h.state().canvas.focused(&h.ctx), "Delete returns focus to the document");
    assert!(h.state().session.clip_history.items().is_empty(), "the item was deleted");
}

fn pane_harness(app: crate::WordApp) -> egui_kittest::Harness<'static, crate::WordApp> {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
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

/// Every pane's body fits a pane at its narrowest body width: button rows wrap and fields narrow. Restrict
/// Editing shows its style list and password fields.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn pane_bodies_fit_the_narrowest_pane() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
    app.run("select.all", serde_json::json!({})).unwrap();
    app.run("edit.copy", serde_json::json!({})).unwrap();
    app.restrict =
        crate::restrict_pane::RestrictPane { open: true, formatting: true, show_styles: true, editing: true, asking: true, ..Default::default() };
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
    let mut h = crate::test_harness::builder().with_size(egui::vec2(400.0, 1200.0)).build_ui_state(
        move |ui, app: &mut crate::WordApp| {
            // Fonts take effect the frame after they are installed.
            if !fonts {
                crate::app::fonts::install_ui_fonts(ui.ctx(), crate::i18n::Lang::EN);
                fonts = true;
                return;
            }
            // `size.pane.least` is only the dock's drag floor (S-L268), not what a body can wrap to: a body must fit
            // the narrowest pane a person actually gets, 200 pt less the 10 pt body margins and the frame's strokes.
            let least = 178.0;
            let mut row = Vec::new();
            for pane in panes {
                let width = ui
                    .allocate_ui_with_layout(vec2(least, 1200.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_max_width(least);
                        super::pane_body(app, ui, pane);
                        ui.min_rect().width()
                    })
                    .inner;
                row.push((pane, width, least));
            }
            *seen.borrow_mut() = row;
        },
        app,
    );
    h.run_steps(3);
    let widths = widths.borrow();
    assert_eq!(widths.len(), panes.len());
    let wide: Vec<_> = widths.iter().filter(|(_, width, least)| *width > *least + 0.5).collect();
    assert!(wide.is_empty(), "wider than the pane: {wide:?}");
}

/// A pane other than Styles floated from its menu comes back floating, at the same rect, after
/// a restart; junk in `panePlaces` reads as the defaults (#1756).
#[test]
fn a_floated_pane_floats_again_after_a_restart() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    app.run("view.navigationPane", serde_json::json!({"value": true})).unwrap();
    let rect = egui::Rect::from_min_size(egui::pos2(30.0, 40.0), vec2(320.0, 400.0));
    super::apply(&mut app, PaneRequest::Place(Pane::Navigation, PanePlace::Floating), false);
    super::apply(&mut app, PaneRequest::FloatRect(Pane::Navigation, rect), false);
    super::apply(&mut app, PaneRequest::Minimize(Pane::Navigation, true), false);
    let state = super::pane_state(&app, Pane::Navigation, false);
    assert_eq!((state.place, state.float_rect, state.minimized), (PanePlace::Floating, Some(rect), true));

    let saved = serde_json::to_vec(&app.prefs()).unwrap();
    let mut next = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    next.apply_prefs(serde_json::from_slice(&saved).unwrap());
    let state = super::pane_state(&next, Pane::Navigation, false);
    assert_eq!((state.place, state.float_rect, state.minimized), (PanePlace::Floating, Some(rect), true));
    assert_eq!(super::pane_state(&next, Pane::Clipboard, false), Pane::Clipboard.default_state(), "other panes keep their defaults");

    let mut junk = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    junk.apply_prefs(
        serde_json::from_str(
            r#"{"panePlaces": {"navigation": {"dock": "top", "float": [1, "x", 3, 4]}, "clipboard": 7, "nope": {}}, "navPane": true}"#,
        )
        .unwrap(),
    );
    assert_eq!(super::pane_state(&junk, Pane::Navigation, false), Pane::Navigation.default_state(), "junk reads as the default");
    assert!(junk.session.view.nav_pane, "the rest of the file still loads");
    let mut odd = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    odd.apply_prefs(serde_json::from_str(r#"{"panePlaces": [1, 2], "navPane": true}"#).unwrap());
    assert!(odd.ui.pane_places.is_empty() && odd.session.view.nav_pane);
}

/// `ui.paneDock` docks or floats any pane and reads its place, as the pane's menu does
/// (#1756); Styles goes through `view.stylesPaneDock`; junk is refused and changes nothing.
#[test]
fn pane_dock_command_docks_floats_and_reads_each_pane() {
    use serde_json::json;
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    assert_eq!(app.run("ui.paneDock", json!({"pane": "navigation"})).unwrap(), json!({"pane": "navigation", "dock": "left"}));
    assert_eq!(app.run("ui.paneDock", json!({"pane": "comments"})).unwrap(), json!({"pane": "comments", "dock": "right"}));
    assert_eq!(app.run("ui.paneDock", json!({"pane": "navigation", "dock": "float"})).unwrap()["dock"], "float");
    assert_eq!(super::pane_state(&app, Pane::Navigation, false).place, PanePlace::Floating);
    app.run("ui.paneDock", json!({"pane": "comments", "dock": "left"})).unwrap();
    assert_eq!(super::pane_state(&app, Pane::Comments, false).place, PanePlace::Docked(PaneEdge::Start));
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
        json!({"pane": "comments", "before": "comments"}),
        json!({"pane": "comments", "before": 7}),
    ] {
        assert!(app.run("ui.paneDock", junk.clone()).is_err(), "{junk}");
    }
    assert_eq!(super::pane_state(&app, Pane::Navigation, false).place, PanePlace::Floating, "junk changed nothing");
}

/// Explicit `left` / `right` stay on the physical window sides in an Arabic (RTL) interface;
/// defaults flip with the reading start. Covers `ui.paneDock` and `view.stylesPaneDock`.
#[test]
fn pane_dock_left_and_right_are_physical_in_rtl() {
    use serde_json::json;
    let arabic = crate::i18n::fixture_lang("ar", "\tHome\tالرئيسية\n");
    crate::i18n::apply_language(arabic);
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    // Navigation's default is the logical Start edge, which is the physical right until set.
    assert_eq!(super::pane_state(&app, Pane::Navigation, true).place, PanePlace::Docked(PaneEdge::Start));
    assert_eq!(app.run("ui.paneDock", json!({"pane": "navigation"})).unwrap()["dock"], "right");
    // Explicit left stays physical left (the End edge here) and is reported as left.
    assert_eq!(app.run("ui.paneDock", json!({"pane": "navigation", "dock": "left"})).unwrap()["dock"], "left");
    assert_eq!(super::pane_state(&app, Pane::Navigation, true).place, PanePlace::Docked(PaneEdge::End));
    assert_eq!(app.run("ui.paneDock", json!({"pane": "navigation"})).unwrap()["dock"], "left");
    assert_eq!(app.run("ui.paneDock", json!({"pane": "comments", "dock": "right"})).unwrap()["dock"], "right");
    assert_eq!(super::pane_state(&app, Pane::Comments, true).place, PanePlace::Docked(PaneEdge::Start));
    // Styles through view.stylesPaneDock — same physical contract.
    assert_eq!(app.run("view.stylesPaneDock", json!({"dock": "left"})).unwrap()["dock"], "left");
    assert_eq!(super::pane_state(&app, Pane::Styles, true).place, PanePlace::Docked(PaneEdge::End));
    assert_eq!(app.run("view.stylesPaneDock", json!({"dock": "right"})).unwrap()["dock"], "right");
    assert_eq!(super::pane_state(&app, Pane::Styles, true).place, PanePlace::Docked(PaneEdge::Start));
    // A pane area request (Dock Left menu / drag) saves the physical side its edge is on.
    super::apply(&mut app, PaneRequest::Place(Pane::Clipboard, PanePlace::Docked(PaneEdge::End)), true);
    assert_eq!(app.ui.pane_places.get("clipboard").and_then(|p| p.dock), Some(super::SavedDock::Left));
    assert_eq!(super::pane_state(&app, Pane::Clipboard, true).place, PanePlace::Docked(PaneEdge::End));
    // A top or bottom request is refused.
    super::apply(&mut app, PaneRequest::Place(Pane::Clipboard, PanePlace::Docked(PaneEdge::Top)), true);
    assert_eq!(app.ui.pane_places.get("clipboard").and_then(|p| p.dock), Some(super::SavedDock::Left));
    crate::i18n::apply_language(crate::i18n::Lang::EN);
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
        // Fonts take effect the frame after they are set, so the first frame only installs them.
        let mut fonts = false;
        let mut h = crate::test_harness::builder().with_size(egui::vec2(400.0, 1200.0)).build_ui_state(
            |ui, app: &mut crate::WordApp| {
                if !fonts {
                    crate::app::fonts::install_ui_fonts(ui.ctx(), crate::i18n::Lang::EN);
                    fonts = true;
                    return;
                }
                super::styles(app, ui);
            },
            app,
        );
        h.run_steps(4);
        // The list owns its scroll and shows 18 rows (`max_rows`), so a row below them is not in the tree
        // until the list is scrolled to it: wheel over the list, as a person does.
        let over_list = h.get_by_label("Manage Styles…").rect().center() + egui::vec2(0.0, 120.0);
        h.event(egui::Event::PointerMoved(over_list));
        h.run_steps(1);
        for _ in 0..40 {
            if h.query_by_label(style).is_some() {
                break;
            }
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
                source: egui::MouseWheelSource::Unknown,
            });
            h.run_steps(2);
        }
        h.get_by_label(style).click_secondary();
        h.run_steps(3);
        h.get_by_label("Delete").accesskit_node().is_disabled()
    }
    assert!(delete_is_disabled("Normal"), "the built-in style's Delete is greyed out");
    assert!(!delete_is_disabled("Callout"), "the custom style's Delete stays enabled");
}

/// #3382: a long comment's card in the Comments pane stops at its capped height, so its buttons
/// and the card after it stay in view.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_long_comment_card_keeps_its_buttons_and_the_next_card_in_view() {
    use egui_kittest::kittest::Queryable;

    let mut app = crate::WordApp::new(
        wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Alpha paragraph one.\n\nBeta paragraph two.")),
        crate::Services::default(),
    );
    app.run("select.text", serde_json::json!({"text": "Alpha"})).unwrap();
    let long = (1..=200).map(|i| format!("Line {i}")).collect::<Vec<_>>().join("\n");
    app.run("review.newComment", serde_json::json!({"text": long})).unwrap();
    app.run("select.text", serde_json::json!({"text": "Beta"})).unwrap();
    app.run("review.newComment", serde_json::json!({"text": "Short note."})).unwrap();
    app.run("view.commentsPane", serde_json::json!({"value": true})).unwrap();
    let h = pane_harness(app);
    let mut go_to: Vec<egui::Rect> = h.get_all_by_label("Go to").map(|n| n.rect()).collect();
    go_to.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
    assert_eq!(go_to.len(), 2, "one Go to button per card");
    let screen = h.ctx.content_rect();
    assert!(go_to.iter().all(|r| screen.contains_rect(*r)), "both cards' buttons are in view: {go_to:?} in {screen:?}");
    assert!(go_to[1].min.y > go_to[0].max.y, "the second card sits below the first: {go_to:?}");
}

/// Numbered headings are listed with the number they show, and numbered body paragraphs in
/// the same list still count (#3418).
#[test]
fn headings_show_their_list_numbers() {
    use std::sync::Arc;
    use wordcraft_doc::props::NumRef;
    use wordcraft_doc::{CharProps, Document, ListKind, Paragraph, para_block};
    let mut doc = Document::new();
    let num = Arc::make_mut(&mut doc.numbering).add_list(ListKind::Legal);
    let para = |text: &str, style: &str, level: Option<u8>| {
        let mut p = Paragraph::with_text(text, CharProps::default()).styled(style);
        p.props.numbering = level.map(|level| NumRef { num, level });
        para_block(p)
    };
    doc.body = vec![
        para("Intro", "Heading1", Some(0)),
        para("Scope", "Heading2", Some(1)),
        para("A numbered body paragraph", "Normal", Some(1)),
        para("Terms", "Heading2", Some(1)),
        para("Method", "Heading1", Some(0)),
        para("Appendix", "Heading1", None),
    ];
    let texts: Vec<String> = nav_headings(&doc).into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(texts, ["1. Intro", "1.1. Scope", "1.3. Terms", "2. Method", "Appendix"]);
}

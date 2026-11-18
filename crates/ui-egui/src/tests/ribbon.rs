use super::*;

/// #3045: on the empty window (no document behind the Start screen) the ribbon is greyed
/// out, but the tab strip works: File reaches the Start screen again, where a document can
/// be made. Disabling the Share / Editing / Comments row disabled the whole line, the tab
/// strip with it: the tabs took clicks that did nothing.
#[test]
fn empty_window_ribbon_tabs_reach_the_start_screen() {
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    a.start_screen();
    let mut h = harness(a);
    h.run_steps(4);
    // Esc (or the back arrow) leaves the Start screen for the empty window, as #1129 has it.
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(!h.state().ui.backstage, "the empty window");
    assert!(!h.state().has_document(), "with no document");
    // File, the first tab, is live and reaches the Start screen.
    h.get_by_label("File").click();
    h.run_steps(4);
    assert!(h.state().ui.backstage && h.state().ui.backstage_page == "home", "File reaches the Start screen");
    assert!(h.query_by_label("Blank document").is_some(), "where a document can be made");
}

/// Issue #67: View › Zoom In from a fit mode zoomed *out*: Page Width showed 163% but Zoom In
/// stepped from the stale manual 100% to 110%.
#[test]
fn view_tab_zoom_steps_from_the_shown_zoom_in_fit_modes() {
    let ctx = egui::Context::default();
    let mut a = app();
    a.run("ui.tab", json!({"tab": "View"})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
    let frame = |a: &mut WordApp| {
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        }
        a.canvas.scale / canvas::PX_PER_PT
    };
    let base = frame(&mut a);
    assert!((base - 1.0).abs() < 1e-3);
    for fit in ["view.pageWidth", "view.onePage", "view.multiplePages"] {
        a.run(fit, json!({})).unwrap();
        let fitted = frame(&mut a);
        assert!((fitted - base).abs() > 0.05, "{fit} changes the zoom");
        assert!((a.session.view.zoom - fitted).abs() < 1e-3, "{fit}: session zoom follows the shown zoom");
        a.run("view.zoomIn", json!({})).unwrap();
        let zin = frame(&mut a);
        assert!(zin > fitted, "{fit}: Zoom In zooms in: {fitted} -> {zin}");
        a.run(fit, json!({})).unwrap();
        frame(&mut a);
        a.run("view.zoomOut", json!({})).unwrap();
        let zout = frame(&mut a);
        assert!(zout < fitted, "{fit}: Zoom Out zooms out: {fitted} -> {zout}");
        // The Zoom dialog opens at the shown zoom.
        a.run(fit, json!({})).unwrap();
        frame(&mut a);
        a.run("ui.dialog", json!({"name": "zoom"})).unwrap();
        assert!(matches!(a.dialog, Some(dialogs::Dialog::Zoom { percent }) if (percent - (fitted * 100.0).round()).abs() < 1.0));
        a.dialog = None;
    }
    a.run("view.zoom100", json!({})).unwrap();
    assert!((frame(&mut a) - 1.0).abs() < 1e-3);
}

/// #4061: a WordArt picked from a style gallery has its text selected and the page gets the
/// keyboard back, so typing replaces the placeholder text at once. The gallery's cells ran
/// their command without handing focus back, so the first keystrokes went nowhere.
#[test]
fn picking_a_wordart_style_hands_the_keyboard_back_to_the_page() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 300.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            // The first pass installs the fonts the gallery's cells are drawn in.
            if ctx.cumulative_pass_nr() > 1 {
                crate::wordart_menus::style_gallery(ui, app, "insert.wordArt");
            }
        },
        app(),
    );
    h.run_steps(3);
    h.state_mut().canvas.want_focus = false;
    // The first cell, "Solid Accent": a click on its middle (the cells carry no label here).
    let at = egui::pos2(30.0, 30.0);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    h.event(button(true));
    h.run_steps(1);
    h.event(button(false));
    h.run_steps(2);
    assert!(canvas::in_text_box(h.state()), "the WordArt's text is selected");
    assert!(h.state().canvas.want_focus, "the page takes the keyboard back");
}

/// Word opens on Home every launch, so the tab `ui.json` holds is not restored; the rest of the
/// preferences come back as saved.
#[test]
fn startup_opens_on_home_not_the_saved_tab() {
    let mut a = app();
    a.ui.tab = "Review".into();
    a.ui.theme = theme::Appearance::Dark;
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved.contains(r#""tab":"Review""#), "{saved}");
    let mut b = app();
    b.apply_prefs(serde_json::from_str(&saved).unwrap());
    assert_eq!(b.ui.tab, DEFAULT_TAB, "the saved tab is not restored");
    assert_eq!(b.ui.theme, theme::Appearance::Dark, "the other preferences are");
}

/// Going File and back to the editor returns to the tab that was showing, contextual tabs
/// included (Word behaves the same way).
#[test]
fn going_to_file_and_back_keeps_the_tab() {
    let mut a = app();
    a.ui_command("ui.tab", &json!({"tab": "Table Design"})).unwrap().unwrap();
    a.ui_command("ui.tab", &json!({"tab": "File"})).unwrap().unwrap();
    assert!(a.ui.backstage, "File opens Backstage, leaving the tab alone");
    a.ui_command("ui.tab", &json!({"tab": "Table Design"})).unwrap().unwrap();
    assert!(!a.ui.backstage && a.ui.tab == "Table Design");
}

/// #2054: clicking a ribbon tab while a footer is being edited scrolled the page a little.
#[test]
fn ribbon_tab_click_does_not_follow_the_caret() {
    let mut a = typed();
    a.canvas.scroll_to_caret = false;
    a.run("ui.tab", json!({"tab": "Design"})).unwrap();
    assert!(!a.canvas.scroll_to_caret, "switching tabs left the view alone");
    a.run("text.insert", json!({"text": "x"})).unwrap();
    assert!(a.canvas.scroll_to_caret, "typing still brings the caret into view");
}

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
    let ctx = crate::test_harness::context();
    let mut a = app();
    a.run("ui.tab", json!({"tab": "View"})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
    let frame = |a: &mut WordApp| {
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            crate::test_harness::pass(&ctx, a, input);
        }
        a.canvas.zoom() as f32
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
        frame(&mut a);
        let open = a.dialog_host.open_dialogs();
        let zoom = open.iter().find(|d| d.key.as_deref() == Some("zoom")).expect("the Zoom dialog is open");
        // Accepting it as it opened applies the percent it opened at: the shown zoom.
        // The held OK files on the first confirm after a shown pass and finishes on a later one (S-L291): the
        // return says nothing, the zoom and the open dialogs do.
        let _ = a.dialog_host.confirm_by_id(&ctx, zoom.id);
        frame(&mut a);
        // The first confirm files the held OK; a later one, after the pass that showed it, finishes the dialog.
        if !a.dialog_host.open_dialogs().is_empty() {
            let _ = a.dialog_host.confirm_by_id(&ctx, zoom.id);
            frame(&mut a);
        }
        assert!((a.session.view.zoom - fitted).abs() < 0.01, "{fit}: the Zoom dialog opens at the shown zoom");
        assert!(a.dialog_host.open_dialogs().is_empty(), "the Zoom dialog closed");
    }
    a.run("view.zoom100", json!({})).unwrap();
    assert!((frame(&mut a) - 1.0).abs() < 1e-3);
}

/// #4061: a WordArt picked from a style gallery has its text selected and the page gets the
/// keyboard back, so typing replaces the placeholder text at once. The gallery's cells ran
/// their command without handing focus back, so the first keystrokes went nowhere.
#[test]
fn picking_a_wordart_style_hands_the_keyboard_back_to_the_page() {
    use egui_kittest::kittest::{By, Queryable};
    let mut a = app();
    a.run("ui.tab", json!({"tab": "Insert"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
    h.run_steps(4);
    // Insert › Decorative Text is the WordArt gallery's opener, under that name in production too.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Decorative Text").click();
    h.run_steps(3);
    // The first style of the WordArt table (`wordcraft_doc::wordart`) is "Solid Accent".
    h.query_all(By::new().label_contains("Solid Accent")).next().expect("the first WordArt style tile").click();
    h.run_steps(3);
    assert!(canvas::in_text_box(h.state()), "the WordArt's text is selected");
    assert!(h.state().canvas.focused(&h.ctx), "the page takes the keyboard back");
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

/// Citations › Provider: the command takes only "zotero"; anything else is an error, never a
/// panic; it changes nothing and adds no undo step. The tab's Provider menu has one item, Zotero,
/// marked as chosen.
#[test]
fn citations_provider_menu_has_zotero_checked() {
    use egui_kittest::kittest::{By, NodeT, Queryable};
    let mut a = app();
    let depth = a.session.undo_depth();
    assert_eq!(a.run("ui.citations.provider", json!({"value": "zotero"})).unwrap(), json!({"provider": "zotero"}));
    for junk in
        [json!({}), json!({"value": "mendeley"}), json!({"value": 7}), json!({"value": null}), json!({"value": ""}), json!({"value": ["zotero"]})]
    {
        assert!(a.run("ui.citations.provider", junk).is_err());
    }
    assert_eq!(a.session.undo_depth(), depth, "not an undo step");
    a.run("ui.tab", json!({"tab": "Citations"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
    h.run_steps(4);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Provider").click();
    h.run_steps(3);
    let items: Vec<_> = h.query_all(By::new().label_contains("Zotero")).collect();
    assert_eq!(items.len(), 1, "the menu has exactly one item, Zotero");
    let checked = items.first().map(|n| n.accesskit_node().toggled().is_some() || n.accesskit_node().is_selected().is_some());
    assert_eq!(checked, Some(true), "and it is marked as the chosen one");
}

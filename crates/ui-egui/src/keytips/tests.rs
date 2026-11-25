use super::*;

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default())
}

fn key_event(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

/// The whole app in a harness with the page focused, as a user's window is.
fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    crate::test_harness::frames(&mut h, 4);
    let ctx = h.ctx.clone();
    // PLAN: `focus_page(ctx)` is canvas-w9's (cv.A10).
    h.state_mut().canvas.focus_page(&ctx);
    crate::test_harness::frames(&mut h, 2);
    h
}

fn bold(h: &mut egui_kittest::Harness<'static, WordApp>) -> bool {
    h.state_mut().session.run("format.state", &serde_json::json!({})).unwrap()["bold"] == true
}

#[test]
fn every_keytip_command_is_registered() {
    // The keytip tables name commands by id; a renamed or removed command must fail here,
    // not silently do nothing when its badge is pressed. Features not built yet are fine
    // when the Word feature catalog lists them (the ribbon shows them too); `ui.*` commands are
    // the app's own (`WordApp::run`).
    let a = app();
    let catalog: Vec<&str> = wordcraft_engine::catalog::entries().into_iter().map(|e| e.3).collect();
    let mut missing = Vec::new();
    for (tab, _) in TAB_LETTERS.iter().chain(&TABLE_TAB_LETTERS) {
        for ctrl in controls(tab) {
            let id = ctrl.command.id.as_str();
            if a.session.registry.get(id).is_none() && !catalog.contains(&id) && !id.starts_with("ui.") {
                missing.push(format!("{tab}: {id}"));
            }
        }
    }
    assert!(missing.is_empty(), "unregistered: {missing:#?}");
}

#[test]
fn keytips_never_reuse_another_commands_shortcut_letter() {
    let a = app();
    let reserved = reserved_letters(&a);
    let folder = craft::i18n::Folder::new("en");
    for (tab, _) in TAB_LETTERS.iter().chain(&TABLE_TAB_LETTERS) {
        let rows = rows(&a, tab);
        let labels: Vec<(&str, Option<&str>)> = rows.iter().map(|r| (r.control.tip, r.letters.as_deref())).collect();
        let badges = craft::keymap::mnemonic::assign_keytips(&labels, &reserved, &folder);
        assert_eq!(badges.len(), rows.len(), "{tab}: one badge per control");
        for (row, badge) in rows.iter().zip(&badges) {
            let id = &row.control.command.id;
            if let Some(own) = &row.letters {
                assert_eq!(badge, own, "{tab}: {id} should keep its shortcut letter {own}");
            } else {
                let first = badge.chars().next().unwrap_or('?');
                assert!(!reserved.contains(&first), "{tab}: badge {badge} collides with another command's shortcut");
            }
        }
    }
}

/// #1697: legacy sequences never start on a tab keytip (table tabs included) or Search, and
/// every one reaches a command or dialog that exists.
#[test]
fn legacy_menu_keys_stay_off_the_ribbon_letters_and_all_resolve() {
    let mut a = app();
    a.run("insert.table", serde_json::json!({"rows": 2, "cols": 2})).unwrap();
    let mut taken: Vec<&str> = badge_tabs(&a).into_iter().map(tab_letter).collect();
    taken.push(SEARCH_LETTER);
    for (keys, command) in legacy() {
        let first = &keys[..1];
        assert!(!taken.iter().any(|l| l.starts_with(first)), "{keys} starts on a ribbon letter");
        assert!(!legacy().iter().any(|(k, _)| *k != *keys && k.starts_with(*keys)), "{keys} hides a longer sequence");
        let mut b = app();
        // The hosted parameter dialogs open nothing before the window's context is known.
        b.ctx = Some(egui::Context::default());
        if command.id == "ui.dialog" {
            b.run(&command.id, command.params.clone()).unwrap();
            assert!(!b.dialog_host.is_empty(), "{keys}: dialog {}", command.params);
        } else {
            assert!(b.session.registry.get(&command.id).is_some(), "{keys}: {}", command.id);
        }
    }
}

/// #1697 review: with no ribbon showing (Focus, Full Screen View) no tab rects are recorded,
/// so the legacy hint hangs under the slim screen bar at the top of the window instead of not
/// showing; with the ribbon showing (normal window or dropped ribbon in Focus) it hangs under
/// the tab row.
#[test]
fn the_legacy_hint_hangs_under_the_slim_bar_without_a_ribbon() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    if cfg!(target_os = "macos") {
        return;
    }
    let trigger_legacy_v = |h: &mut egui_kittest::Harness<'static, WordApp>| {
        h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
        crate::test_harness::frame(h);
        h.input_mut().events.extend([key_event(egui::Key::V, true, egui::Modifiers::NONE), egui::Event::Text("v".into())]);
        crate::test_harness::frames(h, 2);
    };
    let cancel = |h: &mut egui_kittest::Harness<'static, WordApp>| {
        h.input_mut().events.push(key_event(egui::Key::Escape, true, egui::Modifiers::NONE));
        crate::test_harness::frames(h, 2);
    };

    let mut h = harness(app());
    let home_tab = h.get_by_label("Home").rect();
    trigger_legacy_v(&mut h);
    assert!(h.state().keytips_active, "F10, V enters legacy keytip wait with the ribbon showing");
    assert!(h.query_by_label("Old menu keys: Alt, V. Keep typing, or press Esc to cancel.").is_some(), "hint displays pending Alt, V sequence");
    let ribbon_hint = h.get_by_role_and_label(Role::Dialog, "Old menu keys").rect();
    assert!(ribbon_hint.top() >= home_tab.bottom() - 1.0, "legacy hint {ribbon_hint:?} hangs under the ribbon tab row {home_tab:?}");
    cancel(&mut h);
    assert!(!h.state().keytips_active, "Escape ends legacy keytips");
    assert!(h.query_by_role_and_label(Role::Dialog, "Old menu keys").is_none(), "hint closes on Escape");

    // Focus hides the ribbon and shows the slim screen bar at the top.
    h.state_mut().run("view.focus", serde_json::json!({})).unwrap();
    crate::test_harness::frames(&mut h, 3);
    assert!(h.query_by_label("Insert").is_none(), "Focus hides the ribbon");
    let focus_bar = h.get_by_label("Show the ribbon").rect();
    trigger_legacy_v(&mut h);
    assert!(h.state().keytips_active, "F10, V activates legacy keytips in Focus mode");
    let focus_hint = h.get_by_role_and_label(Role::Dialog, "Old menu keys").rect();
    assert!(
        focus_hint.top() >= focus_bar.bottom() - 1.0 && focus_hint.top() < ribbon_hint.top(),
        "Focus legacy hint {focus_hint:?} hangs under the slim screen bar {focus_bar:?} (above ribbon hint {ribbon_hint:?})"
    );
    assert!(focus_hint.width() < 720.0, "Focus hint {focus_hint:?} sizes to its content rather than stretching across the screen bar");
    cancel(&mut h);
    assert!(!h.state().keytips_active, "Escape ends legacy keytips in Focus");
    assert!(h.state().session.view.focus_mode, "Escape cancelling legacy keytips keeps Focus active");

    // When Focus drops the ribbon down, the tab row wins over the screen bar.
    h.state_mut().run("ui.focusRibbon", serde_json::json!({"value": true})).unwrap();
    crate::test_harness::frames(&mut h, 3);
    let dropped_tab = h.get_by_label("Home").rect();
    trigger_legacy_v(&mut h);
    let dropped_hint = h.get_by_role_and_label(Role::Dialog, "Old menu keys").rect();
    assert!(
        dropped_hint.top() >= dropped_tab.bottom() - 1.0 && dropped_hint.top() > focus_hint.top(),
        "dropped-ribbon hint {dropped_hint:?} hangs under the dropped tab row {dropped_tab:?}"
    );
    cancel(&mut h);

    // Full Screen View hides the ribbon and shows the slim screen bar with Search.
    h.state_mut().run("view.focus", serde_json::json!({})).unwrap();
    h.state_mut().run("view.fullScreenView", serde_json::json!({})).unwrap();
    crate::test_harness::frames(&mut h, 3);
    assert!(h.query_by_label("Insert").is_none(), "Full Screen View hides the ribbon");
    let search_bar = h.get_by_label("Search commands and help").rect();
    trigger_legacy_v(&mut h);
    assert!(h.state().keytips_active, "F10, V activates legacy keytips in Full Screen View");
    let fs_hint = h.get_by_role_and_label(Role::Dialog, "Old menu keys").rect();
    assert!(
        fs_hint.top() >= search_bar.bottom() - 1.0 && fs_hint.top() < ribbon_hint.top(),
        "Full Screen View legacy hint {fs_hint:?} hangs under the slim screen bar {search_bar:?} (above ribbon hint {ribbon_hint:?})"
    );
    assert!(fs_hint.width() < 720.0, "Full Screen View hint {fs_hint:?} sizes to its content rather than stretching across the screen bar");
    h.input_mut().events.extend([key_event(egui::Key::U, true, egui::Modifiers::NONE), egui::Event::Text("u".into())]);
    crate::test_harness::frames(&mut h, 3);
    assert!(!h.state().session.view.full_screen_view, "Alt/F10, V, U leaves Full Screen View");
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "no letter typed into the document");
}

#[test]
fn qat_keytips_follow_custom_order_and_skip_disabled_commands() {
    let mut a = app();
    a.run("tools.customizeRibbon", serde_json::json!({"qatAdd": "format.bold"})).unwrap();
    a.run("tools.customizeRibbon", serde_json::json!({"qatAdd": "format.italic"})).unwrap();
    a.run("tools.customizeRibbon", serde_json::json!({"qatMove": {"command": "format.bold", "by": -4}})).unwrap();
    let bold_index = a.session.ribbon.qat().iter().position(|id| *id == "format.bold").unwrap();
    assert_eq!(bold_index, 0);
    let mut h = harness(a);
    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    h.input_mut().events.push(key_event(egui::Key::Num1, true, egui::Modifiers::NONE));
    // The keytip asks for its command in this frame; the runtime runs it at the start of the next.
    crate::test_harness::frames(&mut h, 3);
    assert!(bold(&mut h), "Alt, 1 runs the first button");

    // Bold, then the disabled Undo, then Italic: the digit of Italic among the enabled buttons runs Italic.
    let mut a = app();
    a.run("tools.customizeRibbon", serde_json::json!({"qatAdd": "format.italic"})).unwrap();
    let position = |a: &WordApp, id: &str| a.session.ribbon.qat().iter().position(|q| *q == id).unwrap() as i64;
    let by = position(&a, "edit.undo") + 1 - position(&a, "format.italic");
    a.run("tools.customizeRibbon", serde_json::json!({"qatMove": {"command": "format.italic", "by": by}})).unwrap();
    let mut h = harness(a);
    let ctx = h.ctx.clone();
    let app = h.state();
    let qat = app.session.ribbon.qat();
    let italic = qat.iter().position(|id| *id == "format.italic").unwrap();
    let undo = qat.iter().position(|id| *id == "edit.undo").unwrap();
    assert!(undo < italic, "Undo sits before Italic");
    assert!(!crate::chrome::qat_enabled(&ctx, app, "edit.undo"), "Undo is disabled in an untouched document");
    let digit = qat[..italic].iter().filter(|id| crate::chrome::qat_enabled(&ctx, app, id)).count() + 1;
    let key = egui::Key::from_name(&digit.to_string()).unwrap();
    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    h.input_mut().events.push(key_event(key, true, egui::Modifiers::NONE));
    crate::test_harness::frames(&mut h, 3);
    assert_eq!(h.state_mut().session.run("format.state", &serde_json::json!({})).unwrap()["italic"], true, "the disabled Undo takes no digit");
}

/// #1697 review: Alt, V, U through the whole app (the canvas has the keyboard): Full Screen View
/// turns on and neither letter types into the document.
#[test]
fn legacy_menu_keys_through_the_app_never_type() {
    if cfg!(target_os = "macos") {
        return;
    }
    let mut h = harness(app());
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    h.input_mut().events.extend([key_event(egui::Key::AltLeft, true, egui::Modifiers::NONE), egui::Event::ModifiersChanged(alt)]);
    crate::test_harness::frame(&mut h);
    h.input_mut().events.extend([key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    crate::test_harness::frame(&mut h);
    // PLAN: `WordApp::keytips_active` is app-hub's (rb.A11).
    assert!(h.state().keytips_active, "a bare Alt tap shows the keytips");
    let key = |key| key_event(key, true, egui::Modifiers::NONE);
    h.input_mut().events.extend([key(egui::Key::V), egui::Event::Text("v".into())]);
    crate::test_harness::frame(&mut h);
    h.input_mut().events.extend([key(egui::Key::U), egui::Event::Text("u".into())]);
    crate::test_harness::frames(&mut h, 3);
    assert!(h.state().session.view.full_screen_view, "Alt, V, U: Full Screen View");
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "no letter typed");
}

#[test]
fn keytips_option_persists() {
    let mut a = app();
    a.run("ui.keytips", serde_json::json!({"value": false})).unwrap();
    assert!(a.run("ui.keytips", serde_json::json!({"value": "false"})).is_err());
    let saved = serde_json::to_vec(&a.prefs()).unwrap();
    let mut b = app();
    b.apply_prefs(serde_json::from_slice(&saved).unwrap());
    assert_eq!(b.run("ui.keytips", serde_json::json!({})).unwrap()["enabled"], false);
    b.apply_prefs(serde_json::from_str("{}").unwrap());
    assert!(b.ui.keytips_enabled, "older preferences retain enabled KeyTips");
}

#[test]
fn keytip_cache_reuses_tab_controls_and_updates_on_tab_switch() {
    let mut h = harness(app());
    assert_eq!(h.state().keytip_cache.as_ref().map(|c| c.tab.as_str()), Some("Home"));
    let home_ptr = h.state().keytip_cache.as_ref().unwrap().tab_controls.as_ptr();
    let reserved_ptr = h.state().keytip_cache.as_ref().unwrap().reserved.as_ptr();

    // Idle frames reuse the cached control and reserved-letter buffers without rebuilding.
    crate::test_harness::frames(&mut h, 4);
    assert_eq!(h.state().keytip_cache.as_ref().unwrap().tab_controls.as_ptr(), home_ptr);
    assert_eq!(h.state().keytip_cache.as_ref().unwrap().reserved.as_ptr(), reserved_ptr);

    // Switching tabs rebuilds the cached tab controls while keeping reserved letters intact.
    h.state_mut().run("ui.tab", serde_json::json!({"tab": "Insert"})).unwrap();
    crate::test_harness::frame(&mut h);
    let cache = h.state().keytip_cache.as_ref().unwrap();
    assert_eq!(cache.tab, "Insert");
    // Tab controls plus the trailing Search badge (`SEARCH_LETTER`).
    assert_eq!(cache.tab_controls.len(), controls("Insert").len() + 1);
    assert_eq!(cache.reserved.as_ptr(), reserved_ptr);
}

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
        assert!(!legacy().iter().any(|(k, _)| *k != keys && k.starts_with(keys)), "{keys} hides a longer sequence");
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

<<<<<<< HEAD
/// #1697 review: Alt, V, U through the whole app (the canvas has the keyboard): Full Screen View
/// turns on and neither letter types into the document.
=======
/// #1945: a hidden Quick Access Toolbar has no number badges: a digit after Alt ends the mode
/// and runs none of the toolbar's commands; shown again, the same digit runs the first one.
#[test]
fn hidden_toolbar_has_no_digit_keytips() {
    let mut a = app();
    a.session.ribbon.qat = Some(vec!["format.bold".into()]);
    a.run("ui.quickAccessToolbar", serde_json::json!({"value": false})).unwrap();
    activate(&mut a);
    assert_eq!(a.ui.keytips, Phase::Tabs);
    assert!(handle_key(&mut a, egui::Key::Num1), "the digit is consumed");
    assert_eq!(a.ui.keytips, Phase::Off);
    assert!(a.ui.keytip_prefix.is_empty());
    assert_eq!(a.session.run("format.state", &serde_json::json!({})).unwrap()["bold"], false, "Bold didn't run");
    a.run("ui.quickAccessToolbar", serde_json::json!({"value": true})).unwrap();
    activate(&mut a);
    assert!(handle_key(&mut a, egui::Key::Num1));
    assert_eq!(a.session.run("format.state", &serde_json::json!({})).unwrap()["bold"], true, "shown, 1 runs Bold");
}

#[test]
fn qat_digit_runs_once_and_never_types_into_the_document() {
    let mut a = app();
    a.session.ribbon.qat = Some(vec!["text.insert".into(), "format.bold".into()]);
    a.canvas.focused = true;
    a.canvas.want_focus = true;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    h.input_mut().events.push(key(egui::Key::F10));
    h.step();
    assert_eq!(h.state().ui.keytips, Phase::Tabs);
    h.input_mut().events.extend([key(egui::Key::Num2), egui::Event::Text("2".into())]);
    h.step();
    assert_eq!(h.state().ui.keytips, Phase::Off);
    assert_eq!(h.state_mut().session.run("format.state", &serde_json::json!({})).unwrap()["bold"], true);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty());
    // Normal typing resumes immediately after the command.
    h.input_mut().events.push(egui::Event::Text("hello".into()));
    h.step();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "hello");
    if !cfg!(target_os = "macos") {
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        h.input_mut().events.extend([key_event(egui::Key::AltLeft, true, egui::Modifiers::NONE), egui::Event::ModifiersChanged(alt)]);
        h.step();
        h.input_mut().events.extend([key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        h.step();
        assert_eq!(h.state().ui.keytips, Phase::Tabs);
        h.input_mut().events.extend([key(egui::Key::Num2), egui::Event::Text("2".into())]);
        h.step();
        assert_eq!(h.state_mut().session.run("format.state", &serde_json::json!({})).unwrap()["bold"], false);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "hello");
    }
}

/// Home's Paste, a menu button, shows its keytip whatever the interface language. The keytips
/// table holds the English tips, and `menu_button` matched it with its translated one, which no
/// other language's "Paste" equals or begins with. Pseudo-localization stands in for one.
#[test]
fn a_menu_buttons_keytip_does_not_depend_on_the_language() {
    let a = app();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(3);
    crate::i18n::pseudo::set(true);
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    h.input_mut().events.push(key(egui::Key::F10));
    h.step();
    h.input_mut().events.extend([key(egui::Key::H), egui::Event::Text("h".into())]);
    h.run_steps(3);
    let rects = h.ctx.data(|d| d.get_temp::<Vec<(egui::Rect, String)>>(rects_id())).unwrap_or_default();
    crate::i18n::pseudo::set(false);
    assert_eq!((h.state().ui.tab.as_str(), h.state().ui.keytips), ("Home", Phase::Commands));
    let paste = controls_public("Home").iter().position(|c| c.id == "edit.paste").unwrap();
    let letter = h.ctx.data(|d| d.get_temp::<Vec<String>>(letters_id().with("Home"))).and_then(|l| l.get(paste).cloned()).unwrap();
    assert!(rects.iter().any(|(_, l)| *l == letter), "Paste's {letter} badge is drawn: {rects:?}");
}

/// #1697 review: Alt, V, U through the whole app (the canvas has the keyboard), not only
/// `handle_key`: Full Screen View turns on and neither letter types into the document.
>>>>>>> up/main
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

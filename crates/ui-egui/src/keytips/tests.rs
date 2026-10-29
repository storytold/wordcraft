use super::*;

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default())
}

fn key_event(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

fn input_frame(ctx: &egui::Context, app: &mut WordApp, events: Vec<egui::Event>) {
    ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| logic(app, ui.ctx())).drop_without_applying_deltas();
}

#[test]
fn native_alt_taps_toggle_once_in_either_event_order() {
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    // Key-up can precede ModifiersChanged and still carry alt=true. Some backends send
    // only modifier events; a fast tap can also fit entirely in a single frame.
    for key in [None, Some(egui::Key::AltLeft), Some(egui::Key::AltRight)] {
        for modifiers_first in [false, true] {
            for same_frame in [false, true] {
                let ctx = egui::Context::default();
                let mut a = app();
                let mut press = vec![egui::Event::ModifiersChanged(alt)];
                let mut release = vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
                if let Some(key) = key {
                    if modifiers_first {
                        press.push(key_event(key, true, alt));
                        release.push(key_event(key, false, egui::Modifiers::NONE));
                    } else {
                        press.insert(0, key_event(key, true, egui::Modifiers::NONE));
                        release.insert(0, key_event(key, false, alt));
                    }
                }
                for expected in [if cfg!(target_os = "macos") { Phase::Off } else { Phase::Tabs }, Phase::Off] {
                    if same_frame {
                        input_frame(&ctx, &mut a, press.iter().chain(&release).cloned().collect());
                    } else {
                        input_frame(&ctx, &mut a, press.clone());
                        input_frame(&ctx, &mut a, release.clone());
                    }
                    assert_eq!(a.ui.keytips, expected, "key={key:?}, modifiers_first={modifiers_first}, same_frame={same_frame}");
                    assert!(!a.ui.alt_pressed);
                }
            }
        }
    }
}

#[test]
fn native_alt_chords_and_interrupted_taps_do_not_activate() {
    let ctx = egui::Context::default();
    let mut a = app();
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    for modifier in [egui::Modifiers::CTRL, egui::Modifiers::SHIFT, egui::Modifiers::MAC_CMD] {
        input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(modifier | alt)]);
        // Releasing the other modifier first must not turn this into a bare Alt tap.
        input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(alt)]);
        input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        assert_eq!(a.ui.keytips, Phase::Off);
    }
    for event in [
        key_event(egui::Key::ArrowDown, true, alt),
        egui::Event::PointerButton { pos: egui::pos2(400.0, 300.0), button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
    ] {
        input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(alt)]);
        input_frame(&ctx, &mut a, vec![event]);
        // A repeated key-down must not forget the key/mouse chord.
        input_frame(&ctx, &mut a, vec![key_event(egui::Key::AltLeft, true, alt)]);
        input_frame(&ctx, &mut a, vec![key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        assert_eq!(a.ui.keytips, Phase::Off);
    }
    input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(alt)]);
    ctx.run_ui(egui::RawInput { focused: false, events: vec![egui::Event::WindowFocused(false)], ..Default::default() }, |ui| {
        logic(&mut a, ui.ctx())
    })
    .drop_without_applying_deltas();
    input_frame(&ctx, &mut a, vec![key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    assert_eq!(a.ui.keytips, Phase::Off, "returning after Alt+Tab must not arm keytips");
    input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(alt)]);
    a.ui.backstage = true;
    input_frame(&ctx, &mut a, vec![]);
    a.ui.backstage = false;
    input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    assert_eq!(a.ui.keytips, Phase::Off, "suppression cancels the pending Alt tap");
}

#[test]
fn tab_letters_match_word() {
    // Observed in Word for Windows (Microsoft 365): Alt shows these over the tabs.
    assert_eq!(tab_letter("File"), "F");
    assert_eq!(tab_letter("Home"), "H");
    assert_eq!(tab_letter("Insert"), "N");
    assert_eq!(tab_letter("Design"), "G");
    assert_eq!(tab_letter("Layout"), "P");
    assert_eq!(tab_letter("References"), "S");
    assert_eq!(tab_letter("Mailings"), "M");
    assert_eq!(tab_letter("Review"), "R");
    assert_eq!(tab_letter("View"), "W");
    assert_eq!(tab_letter("Help"), "Y");
    assert_eq!(tab_letter("Table Design"), "JT");
    assert_eq!(tab_letter("Table Layout"), "JL");
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
            if let Target::Cmd { id, .. } = ctrl.target
                && a.session.registry.get(id).is_none()
                && !catalog.contains(&id)
                && !id.starts_with("ui.")
            {
                missing.push(format!("{tab}: {id}"));
            }
        }
    }
    assert!(missing.is_empty(), "unregistered: {missing:#?}");
}

#[test]
fn letters_are_unique_inside_every_tab() {
    let a = app();
    let (tabs, cmds) = letters(&a);
    for (_, ls) in &cmds {
        let mut seen = Vec::new();
        for l in ls {
            assert!(!seen.contains(l), "duplicate badge letter {l}");
            seen.push(l.clone());
        }
    }
    let mut tl = Vec::new();
    for (tab, l) in &tabs {
        assert!(!l.is_empty(), "tab {tab} has no badge");
        assert!(!tl.contains(l), "duplicate tab letter {l}");
        tl.push(l.clone());
    }
}

#[test]
fn keytips_never_reuse_another_commands_shortcut_letter() {
    let a = app();
    let reserved = reserved_letters(&a);
    let (_, cmds) = letters(&a);
    for (tab, ls) in &cmds {
        let ctrls = controls(tab);
        let ids: Vec<String> = ctrls
            .iter()
            .map(|c| match &c.target {
                Target::Cmd { id, .. } => (*id).to_string(),
                _ => "ui.dialog".to_string(),
            })
            .collect();
        for (i, l) in ls.iter().enumerate() {
            let id = ids.get(i).map(|s| s.as_str()).unwrap_or("");
            if let Some(sc) = shortcut_letter(&a, id) {
                assert_eq!(l, &sc.to_string(), "{tab}: {id} should keep its shortcut letter {sc}");
            } else if reserved.contains(&l.chars().next().unwrap_or('?')) {
                panic!("{tab}: badge {l} collides with another command's shortcut");
            }
        }
    }
}

/// Alt+drag selects a column (#237); releasing Alt afterwards must not open keytips. A bare
/// Alt tap still does (off macOS, where Option types accents).
#[test]
fn alt_click_or_drag_does_not_arm_keytips_on_release() {
    let ctx = egui::Context::default();
    let mut a = app();
    a.ui.backstage = false;
    a.dialog = None;
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    let key = |pressed, modifiers| egui::Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed, repeat: false, modifiers };
    let pos = egui::pos2(400.0, 300.0);
    let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        let input = egui::RawInput { events, ..Default::default() };
        ctx.run_ui(input, |ui| logic(a, ui.ctx())).drop_without_applying_deltas();
    };
    // Alt down, drag with the mouse over several frames, Alt up.
    frame(&mut a, vec![key(true, alt)]);
    frame(&mut a, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: alt }]);
    frame(&mut a, vec![egui::Event::PointerMoved(pos + egui::vec2(60.0, 40.0))]);
    frame(&mut a, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: alt }]);
    frame(&mut a, vec![key(false, egui::Modifiers::NONE)]);
    assert_eq!(a.ui.keytips, Phase::Off, "Alt+drag then Alt up leaves keytips off");
    // Alt held while a key is pressed in a later frame: also a chord.
    frame(&mut a, vec![key(true, alt)]);
    frame(&mut a, vec![egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: alt }]);
    frame(&mut a, vec![key(false, egui::Modifiers::NONE)]);
    assert_eq!(a.ui.keytips, Phase::Off, "Alt+key then Alt up leaves keytips off");
    // A bare tap (press and release in separate frames) arms them.
    frame(&mut a, vec![key(true, alt)]);
    frame(&mut a, vec![key(false, egui::Modifiers::NONE)]);
    let tap = if cfg!(target_os = "macos") { Phase::Off } else { Phase::Tabs };
    assert_eq!(a.ui.keytips, tap);
}

/// #1697: Alt, V, U turns on Full Screen View, as in Word for Windows; Alt, V waits with a hint
/// instead of tab badges; a letter that leads nowhere is ignored; Esc ends the sequence and
/// Alt goes back to the tab badges.
#[test]
fn legacy_menu_keys() {
    let mut a = app();
    let keys = |a: &mut WordApp, ks: &[egui::Key]| {
        activate(a);
        for k in ks {
            assert!(handle_key(a, *k));
        }
    };
    if cfg!(target_os = "macos") {
        // Off on macOS: F10's badges take only the ribbon's letters.
        keys(&mut a, &[egui::Key::V]);
        assert_eq!(a.ui.keytips, Phase::Off);
        return;
    }
    keys(&mut a, &[egui::Key::V]);
    assert_eq!((a.ui.keytips, a.ui.keytip_prefix.as_str()), (Phase::Legacy, "V"));
    assert!(handle_key(&mut a, egui::Key::S), "S leads nowhere after V");
    assert_eq!((a.ui.keytips, a.ui.keytip_prefix.as_str()), (Phase::Legacy, "V"), "and is ignored");
    assert!(handle_key(&mut a, egui::Key::U));
    assert_eq!(a.ui.keytips, Phase::Off);
    assert!(a.ui.keytip_prefix.is_empty());
    assert!(a.session.view.full_screen_view, "Alt, V, U: Full Screen View");
    // Three letters: Alt, A, I, T opens Insert Table.
    keys(&mut a, &[egui::Key::A, egui::Key::I]);
    assert_eq!((a.ui.keytips, a.ui.keytip_prefix.as_str()), (Phase::Legacy, "AI"));
    assert!(handle_key(&mut a, egui::Key::T));
    assert!(matches!(a.dialog, Some(crate::dialogs::Dialog::InsertTable { .. })));
    a.dialog = None;
    // Esc ends the whole sequence; Alt goes back to the tab badges.
    keys(&mut a, &[egui::Key::A, egui::Key::I, egui::Key::Escape]);
    assert_eq!(a.ui.keytips, Phase::Off);
    assert!(a.ui.keytip_prefix.is_empty());
    keys(&mut a, &[egui::Key::E, egui::Key::AltLeft]);
    assert_eq!(a.ui.keytips, Phase::Tabs);
    assert!(a.ui.keytip_prefix.is_empty());
    // The ribbon comes first: W is still the View tab.
    a.ui.keytips = Phase::Off;
    keys(&mut a, &[egui::Key::W]);
    assert_eq!((a.ui.tab.as_str(), a.ui.keytips), ("View", Phase::Commands));
}

/// #1697 review: with no ribbon showing (Focus, Full Screen View) no tab rects are recorded,
/// so the hint hangs under the slim bar at the top of the window instead of not showing.
#[test]
fn the_legacy_hint_hangs_under_the_slim_bar_without_a_ribbon() {
    let window = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(1440.0, 900.0));
    let anchor = legacy_hint_anchor(None, window);
    assert_eq!(anchor.left_bottom(), egui::pos2(0.0, crate::chrome::SCREEN_BAR_HEIGHT));
    let tabs = Rect::from_min_max(egui::pos2(10.0, 40.0), egui::pos2(600.0, 64.0));
    assert_eq!(legacy_hint_anchor(Some(tabs), window), tabs, "under the tab row when the ribbon shows");
}

/// #1697: legacy sequences never start on a tab keytip (table tabs included) or Search, and
/// every one reaches a command or dialog that exists.
#[test]
fn legacy_menu_keys_stay_off_the_ribbon_letters_and_all_resolve() {
    let mut a = app();
    a.run("insert.table", serde_json::json!({"rows": 2, "cols": 2})).unwrap();
    let mut taken: Vec<&str> = badge_tabs(&a).into_iter().map(tab_letter).collect();
    taken.push(SEARCH_LETTER);
    for (keys, target) in legacy() {
        let first = &keys[..1];
        assert!(!taken.contains(&first), "{keys} starts on a ribbon letter");
        assert!(!legacy().iter().any(|(k, _)| *k != keys && k.starts_with(keys)), "{keys} hides a longer sequence");
        let mut b = app();
        match target {
            Target::Cmd { id, .. } => assert!(b.session.registry.get(id).is_some(), "{keys}: {id}"),
            Target::Dialog { name } => {
                Target::Dialog { name }.run(&mut b);
                assert!(b.dialog.is_some(), "{keys}: dialog {name}");
            }
            Target::Tab(_) => panic!("{keys}: a tab"),
        }
    }
}

#[test]
fn keytip_state_machine() {
    let mut a = app();
    a.ui.backstage = false;
    a.dialog = None;
    a.canvas.focused = false;
    activate(&mut a);
    assert_eq!(a.ui.keytips, Phase::Tabs);
    assert!(handle_key(&mut a, egui::Key::Escape));
    assert_eq!(a.ui.keytips, Phase::Off);
    activate(&mut a);
    assert!(handle_key(&mut a, egui::Key::H));
    assert_eq!(a.ui.tab, "Home");
    assert_eq!(a.ui.keytips, Phase::Commands);
    assert!(handle_key(&mut a, egui::Key::F));
    assert_eq!(a.ui.keytips, Phase::Off);
    // Alt, F: the Backstage, and keytips end.
    a.ui.backstage = false;
    activate(&mut a);
    assert!(handle_key(&mut a, egui::Key::F));
    assert!(a.ui.backstage, "Alt, F opens File");
    assert_eq!(a.ui.keytips, Phase::Off);
    // Alt, W: View; Alt, R: Review (Word's letters, not V and S).
    a.ui.backstage = false;
    activate(&mut a);
    assert!(handle_key(&mut a, egui::Key::W));
    assert_eq!((a.ui.tab.as_str(), a.ui.keytips), ("View", Phase::Commands));
    a.ui.keytips = Phase::Off;
    activate(&mut a);
    assert!(handle_key(&mut a, egui::Key::R));
    assert_eq!(a.ui.tab, "Review");
}

/// Word's contextual table tabs take two letters (#1183): Alt, J, T is Table Design and
/// Alt, J, L is Table Layout. J alone waits for the second letter.
#[test]
fn alt_j_t_and_alt_j_l_open_the_table_tabs() {
    let mut a = app();
    a.run("insert.table", serde_json::json!({"rows": 2, "cols": 2})).unwrap();
    assert!(crate::ribbon::in_table_public(&a));
    for (second, tab) in [(egui::Key::T, "Table Design"), (egui::Key::L, "Table Layout")] {
        a.run("ui.tab", serde_json::json!({"tab": "Home"})).unwrap();
        activate(&mut a);
        assert!(handle_key(&mut a, egui::Key::J));
        assert_eq!((a.ui.tab.as_str(), a.ui.keytips), ("Home", Phase::Tabs), "J waits for the second letter");
        assert!(handle_key(&mut a, second));
        assert_eq!((a.ui.tab.as_str(), a.ui.keytips), (tab, Phase::Commands));
        assert!(a.ui.keytip_prefix.is_empty());
        assert!(!letters_for(&a, tab).is_empty(), "{tab} badges its controls");
        a.ui.keytips = Phase::Off;
    }
    // J then a letter no table tab uses ends keytips without switching tabs.
    a.run("ui.tab", serde_json::json!({"tab": "Home"})).unwrap();
    activate(&mut a);
    handle_key(&mut a, egui::Key::J);
    handle_key(&mut a, egui::Key::X);
    assert_eq!((a.ui.tab.as_str(), a.ui.keytips), ("Home", Phase::Off));
    assert!(a.ui.keytip_prefix.is_empty());
    // Outside a table there is no J tab.
    let mut b = app();
    activate(&mut b);
    handle_key(&mut b, egui::Key::J);
    assert_eq!(b.ui.keytips, Phase::Off);
}

#[test]
fn qat_keytips_follow_custom_order_and_skip_disabled_commands() {
    let mut a = app();
    a.run("tools.customizeRibbon", serde_json::json!({"qatAdd": "format.bold"})).unwrap();
    a.run("tools.customizeRibbon", serde_json::json!({"qatAdd": "format.italic"})).unwrap();
    a.run("tools.customizeRibbon", serde_json::json!({"qatMove": {"command": "format.bold", "by": -4}})).unwrap();
    let bold_index = a.session.ribbon.qat().iter().position(|id| *id == "format.bold").unwrap();
    assert_eq!(bold_index, 0);
    activate(&mut a);
    handle_key(&mut a, egui::Key::Num1);
    assert_eq!(a.session.run("format.state", &serde_json::json!({})).unwrap()["bold"], true);
    assert_eq!(a.ui.keytips, Phase::Off);

    let mut b = app();
    let before = b.session.doc.clone();
    activate(&mut b);
    handle_key(&mut b, egui::Key::Num2); // Undo is disabled in an untouched document.
    assert_eq!(b.session.doc, before);
    assert!(!b.session.can_redo());
    assert_eq!(b.ui.keytips, Phase::Off);
}

#[test]
fn long_qat_badges_wait_for_the_complete_number_and_escape_cancels() {
    let mut a = app();
    let mut commands: Vec<String> = a.session.registry.all().iter().filter(|s| s.id != "format.bold").take(19).map(|s| s.id.to_string()).collect();
    commands.insert(9, "format.bold".into());
    a.session.ribbon.qat = Some(commands);
    activate(&mut a);
    handle_key(&mut a, egui::Key::Num0);
    handle_key(&mut a, egui::Key::Num0);
    assert_eq!(a.ui.keytips, Phase::Tabs);
    assert_eq!(a.session.run("format.state", &serde_json::json!({})).unwrap()["bold"], false);
    handle_key(&mut a, egui::Key::Num1); // 001: first button after the first nine.
    assert_eq!(a.session.run("format.state", &serde_json::json!({})).unwrap()["bold"], true);
    assert_eq!(a.ui.keytips, Phase::Off);
    activate(&mut a);
    handle_key(&mut a, egui::Key::Num0);
    handle_key(&mut a, egui::Key::Escape);
    assert!(a.ui.keytip_prefix.is_empty());
    assert_eq!(a.ui.keytips, Phase::Off);

    for count in [9, 10, 18, 19, 100, 200] {
        let labels: Vec<_> = (0..count).map(|i| qat_label(i, count)).collect();
        for (i, label) in labels.iter().enumerate() {
            assert!(!labels.iter().enumerate().any(|(j, other)| i != j && other.starts_with(label)), "ambiguous label {label} in {count} buttons");
        }
    }
}

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

/// #1697 review: Alt, V, U through the whole app (the canvas has the keyboard), not only
/// `handle_key`: Full Screen View turns on and neither letter types into the document.
#[test]
fn legacy_menu_keys_through_the_app_never_type() {
    if cfg!(target_os = "macos") {
        return;
    }
    let mut a = app();
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
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    h.input_mut().events.extend([key_event(egui::Key::AltLeft, true, egui::Modifiers::NONE), egui::Event::ModifiersChanged(alt)]);
    h.step();
    h.input_mut().events.extend([key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    h.step();
    assert_eq!(h.state().ui.keytips, Phase::Tabs);
    let key = |key| key_event(key, true, egui::Modifiers::NONE);
    h.input_mut().events.extend([key(egui::Key::V), egui::Event::Text("v".into())]);
    h.step();
    assert_eq!((h.state().ui.keytips, h.state().ui.keytip_prefix.as_str()), (Phase::Legacy, "V"));
    h.input_mut().events.extend([key(egui::Key::U), egui::Event::Text("u".into())]);
    h.run_steps(3);
    assert_eq!(h.state().ui.keytips, Phase::Off);
    assert!(h.state().session.view.full_screen_view, "Alt, V, U: Full Screen View");
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "no letter typed");
}

#[test]
fn keytips_option_persists_and_disabled_keytips_leave_f10_alone() {
    let mut a = app();
    activate(&mut a);
    a.run("ui.keytips", serde_json::json!({"value": false})).unwrap();
    assert_eq!(a.ui.keytips, Phase::Off);
    assert!(a.run("ui.keytips", serde_json::json!({"value": "false"})).is_err());
    let saved = serde_json::to_vec(&a.prefs()).unwrap();
    let mut b = app();
    b.apply_prefs(serde_json::from_slice(&saved).unwrap());
    activate(&mut b);
    assert_eq!(b.ui.keytips, Phase::Off);
    assert_eq!(b.run("ui.keytips", serde_json::json!({})).unwrap()["enabled"], false);
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::F10,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| logic(&mut b, ui.ctx()),
    );
    output.textures_delta.clear();
    assert_eq!(b.ui.keytips, Phase::Off);
    b.apply_prefs(serde_json::from_str("{}").unwrap());
    assert!(b.ui.keytips_enabled, "older preferences retain enabled KeyTips");
}

/// Alt, then Q opens Search like Alt+Q (#819); no tab owns Q.
#[test]
fn alt_then_q_opens_search() {
    let mut a = app();
    a.ui.backstage = false;
    a.dialog = None;
    assert!(TAB_LETTERS.iter().all(|(_, l)| *l != SEARCH_LETTER));
    activate(&mut a);
    assert!(handle_key(&mut a, egui::Key::Q));
    assert_eq!(a.ui.keytips, Phase::Off);
    assert!(matches!(a.dialog, Some(crate::dialogs::Dialog::Commands { .. })), "{:?}", a.dialog.as_ref().map(|d| d.name()));
}

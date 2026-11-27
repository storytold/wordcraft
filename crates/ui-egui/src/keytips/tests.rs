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

fn painted_text_rects(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(children) => {
                for child in children {
                    walk(child, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
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
        let tab_controls = KeytipCache::build_tab_controls(&a, tab);
        let labels: Vec<(&str, Option<&str>)> = tab_controls.iter().map(|c| (c.label, c.letters)).collect();
        let badges = craft::keymap::mnemonic::assign_keytips(&labels, &reserved, &folder);
        assert_eq!(badges.len(), tab_controls.len(), "{tab}: one badge per control");
        for (control, badge) in tab_controls.iter().zip(&badges) {
            let id = &control.command.id;
            assert!(!badge.is_empty(), "{tab}: {id} must receive a non-empty keytip badge");
            if let Some(own) = control.letters {
                assert_eq!(badge, own, "{tab}: {id} should keep its shortcut letter {own}");
            } else {
                let first = badge.chars().next().unwrap_or('?');
                assert!(!reserved.contains(&first), "{tab}: badge {badge} collides with another command's shortcut");
            }
        }
    }
}

/// Home's Paste, a menu button, shows its keytip whatever the interface language. The keytips
/// table holds the English tips, and `menu_button` previously matched it with its translated one,
/// which no other language's "Paste" equals or begins with. Pseudo-localization stands in for one.
#[test]
fn a_menu_buttons_keytip_does_not_depend_on_the_language() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;

    let a = app();
    let reserved = reserved_letters(&a);
    let cache = KeytipCache::new(&a, "Home");
    let paste_idx = cache.tab_controls.iter().position(|c| c.command.id == "edit.paste").expect("Home tab has Paste");
    let labels: Vec<(&str, Option<&str>)> = cache.tab_controls.iter().map(|c| (c.label, c.letters)).collect();

    let english_folder = craft::i18n::Folder::new("en");
    let english_badges = craft::keymap::mnemonic::assign_keytips(&labels, &reserved, &english_folder);
    let paste_badge = &english_badges[paste_idx];
    assert!(!paste_badge.is_empty(), "Paste receives a keytip badge");

    for lang in ["de", "fr", "es", "tr", "pseudo", "ja", "zh", "pt-BR"] {
        let folder = craft::i18n::Folder::new(lang);
        let badges = craft::keymap::mnemonic::assign_keytips(&labels, &reserved, &folder);
        assert_eq!(&badges[paste_idx], paste_badge, "language={lang}: Paste badge must stay {paste_badge} regardless of language");
    }

    let mut h = harness(app());
    h.ctx.add_glyph_rasterizer(egui::GlyphRasterizer::new("wordcraft::keytips::pseudo_markers", |request| {
        matches!(request.cluster, "⟦" | "⟧").then(|| egui::RasterizedGlyph {
            bitmap: egui::GlyphBitmap {
                image: egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
                offset_px: egui::Vec2::ZERO,
                is_color: false,
            },
            advance_px: request.font_size_px.max(1.0),
        })
    }));
    h.state_mut().ui.language = "pseudo".into();
    crate::test_harness::frames(&mut h, 2);

    let paste_rect = h.get_by_role_and_label(Role::Button, "⟦Paste⟧").rect();
    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    h.input_mut().events.extend([key_event(egui::Key::H, true, egui::Modifiers::NONE), egui::Event::Text("h".into())]);
    crate::test_harness::frames(&mut h, 3);
    let painted = painted_text_rects(&h.output().shapes);
    crate::i18n::apply_language(crate::i18n::Lang::EN);

    assert_eq!(h.state().ui.tab.as_str(), "Home");
    assert!(h.state().keytips_active, "F10, H activates Home command keytips");
    assert!(
        painted.iter().any(|(text, rect)| text == paste_badge && paste_rect.expand(8.0).contains(rect.center())),
        "Paste's {paste_badge:?} badge is drawn over Paste {paste_rect:?} under pseudo-localization: {painted:?}"
    );
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

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(children) => {
                for child in children {
                    walk(child, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

#[test]
fn long_qat_badges_wait_for_the_complete_number_and_escape_cancels() {
    // Collect genuinely enabled commands from a live harness context (where `frame_input` has
    // published command states) so all 20 QAT buttons remain active during the test.
    let mut h = harness(app());
    let commands: Vec<String> = {
        let ctx = h.ctx.clone();
        let a = h.state();
        let mut ids: Vec<String> = a
            .session
            .registry
            .all()
            .iter()
            .filter(|s| s.id != "format.bold" && s.id != "format.italic" && crate::chrome::qat_enabled(&ctx, a, s.id))
            .take(18)
            .map(|s| s.id.to_string())
            .collect();
        ids.insert(0, "format.italic".into());
        ids.insert(9, "format.bold".into());
        ids
    };
    assert_eq!(commands.len(), 20, "expected 20 enabled QAT commands");
    h.state_mut().session.ribbon.qat = Some(commands);
    crate::test_harness::frames(&mut h, 2);
    {
        let ctx = h.ctx.clone();
        let a = h.state();
        let enabled_count = a.session.ribbon.qat().iter().filter(|id| crate::chrome::qat_enabled(&ctx, a, id)).count();
        assert_eq!(enabled_count, 20, "all 20 QAT commands stay enabled");
    }

    let digit = |key, ch: &str| [key_event(key, true, egui::Modifiers::NONE), egui::Event::Text(ch.into())];

    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    assert!(h.state().keytips_active);
    let shown = painted_texts(&h.output().shapes);
    assert!(shown.iter().any(|t| t == "001"), "20-button QAT paints 001 for the 10th button: {shown:?}");

    h.input_mut().events.extend(digit(egui::Key::Num0, "0"));
    crate::test_harness::frame(&mut h);
    assert!(h.state().keytips_active, "first 0 waits for the rest of the multi-digit badge");
    h.input_mut().events.extend(digit(egui::Key::Num0, "0"));
    crate::test_harness::frame(&mut h);
    assert!(h.state().keytips_active, "second 0 still waits for the final digit of 001");
    assert!(!bold(&mut h), "incomplete prefix 00 must not fire Bold");
    let narrowed = painted_texts(&h.output().shapes);
    assert!(narrowed.iter().any(|t| t == "001"), "001 remains painted while waiting on prefix 00");
    assert!(!narrowed.iter().any(|t| t == "010"), "non-matching multi-digit badges are hidden under prefix 00");

    h.input_mut().events.extend(digit(egui::Key::Num1, "1")); // 001: first button after the first nine.
    crate::test_harness::frames(&mut h, 3);
    assert!(bold(&mut h), "001 runs Bold");
    assert!(!h.state().keytips_active, "completing 001 ends keytips");
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "QAT digits never type into the document");

    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    assert!(h.state().keytips_active);
    h.input_mut().events.extend(digit(egui::Key::Num0, "0"));
    crate::test_harness::frame(&mut h);
    assert!(h.state().keytips_active);
    h.input_mut().events.push(key_event(egui::Key::Escape, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    assert!(!h.state().keytips_active, "Escape cancels a partial multi-digit prefix");

    // Reopening keytips and pressing 1 fires button 1 (Italic), proving the cancelled "0" prefix was cleared.
    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    assert!(h.state().keytips_active);
    h.input_mut().events.extend(digit(egui::Key::Num1, "1"));
    crate::test_harness::frames(&mut h, 3);
    assert!(!h.state().keytips_active);
    assert_eq!(
        h.state_mut().session.run("format.state", &serde_json::json!({})).unwrap()["italic"],
        true,
        "after Escape, F10 then 1 runs the first button"
    );
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "no keytip digit typed into the document");

    // Verify production `craft::menu::Keytips` assigns distinct, prefix-free digit badges across
    // single- and multi-digit QAT lengths without any test-only label helper.
    for count in [9, 10, 18, 19, 100, 200] {
        let digits: Vec<CommandRef> = (0..count).map(|i| CommandRef::new(format!("test.qat.{i}"))).collect();
        let region_id = egui::Id::unique(("test_qat_digit_badges", count));
        let ctx = crate::test_harness::context();
        let run_pass = |events: Vec<egui::Event>| {
            craft::ui::shell::embed::before_input(&ctx, |_| craft::ui::runtime::frame::FrameInput::new(craft::ui::runtime::env::RootEnv::default()));
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2000.0, 2000.0))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (_, recorded) = inspect::record(ui, region_id, |ui| {
                        for row in digits.chunks(20) {
                            ui.horizontal(|ui| {
                                for cmd in row {
                                    craft::ui::buttons::IconButton::command(craft::behavior::IconId::new("bold"), cmd.clone()).show(ui);
                                }
                            });
                        }
                    });
                    let places: Vec<Arc<Recorded>> = recorded.into_iter().collect();
                    let _ = Keytips::new(true).digits(&digits).places(&places).show_over(ui);
                },
            )
        };
        run_pass(Vec::new()).drop_without_applying_deltas();
        let shown = run_pass(vec![key_event(egui::Key::F10, true, egui::Modifiers::NONE)]);
        let labels = painted_texts(&shown.shapes);
        shown.drop_without_applying_deltas();
        assert_eq!(labels.len(), count, "expected one painted digit badge per QAT command for count={count}: {labels:?}");
        for (i, label) in labels.iter().enumerate() {
            assert!(!label.is_empty() && label.chars().all(|c| c.is_ascii_digit()), "non-digit badge {label:?} at {i} for count={count}");
            assert!(
                !labels.iter().enumerate().any(|(j, other)| i != j && other.starts_with(label.as_str())),
                "ambiguous label {label} in {count} buttons"
            );
        }
    }
}

/// #819: Alt (or F10), then Q opens Search in the title bar, ends keytips, and never types into
/// the document; while the Backstage is open, Q does not open Search.
#[test]
fn alt_then_q_opens_search() {
    let mut h = harness(app());
    assert!(!crate::chrome::search::is_open(h.state()), "Search starts closed");

    // F10, then Q works on every platform (including macOS, where bare Option types characters).
    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    assert!(h.state().keytips_active, "F10 activates keytips");

    h.input_mut().events.extend([key_event(egui::Key::Q, true, egui::Modifiers::NONE), egui::Event::Text("q".into())]);
    crate::test_harness::frames(&mut h, 2);
    assert!(!h.state().keytips_active, "Q ends keytips");
    assert!(crate::chrome::search::is_open(h.state()), "F10, Q opens Search");
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "Q does not type into the document");

    // On non-macOS, a bare Alt tap followed by Q opens Search without typing into the document.
    if !cfg!(target_os = "macos") {
        h.state_mut().chrome.search = false;
        let ctx = h.ctx.clone();
        h.state_mut().canvas.focus_page(&ctx);
        crate::test_harness::frame(&mut h);

        let alt = egui::Modifiers { alt: true, ..Default::default() };
        h.input_mut().events.extend([key_event(egui::Key::AltLeft, true, egui::Modifiers::NONE), egui::Event::ModifiersChanged(alt)]);
        crate::test_harness::frame(&mut h);
        h.input_mut().events.extend([key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        crate::test_harness::frame(&mut h);
        assert!(h.state().keytips_active, "a bare Alt tap activates keytips");

        h.input_mut().events.extend([key_event(egui::Key::Q, true, egui::Modifiers::NONE), egui::Event::Text("q".into())]);
        crate::test_harness::frames(&mut h, 2);
        assert!(!h.state().keytips_active, "Q ends keytips after bare Alt");
        assert!(crate::chrome::search::is_open(h.state()), "Alt, Q opens Search");
        assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "Alt, Q does not type into the document");
    }

    // With Backstage open, Q does not open Search behind it.
    h.state_mut().chrome.search = false;
    h.state_mut().run("ui.backstage", serde_json::json!({"value": true})).unwrap();
    crate::test_harness::frame(&mut h);
    assert!(h.state().ui.backstage, "Backstage is open");

    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    h.input_mut().events.extend([key_event(egui::Key::Q, true, egui::Modifiers::NONE), egui::Event::Text("q".into())]);
    crate::test_harness::frames(&mut h, 2);
    assert!(!crate::chrome::search::is_open(h.state()), "Q does not open Search while Backstage is open");
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "Q does not open the Search dialog while Backstage is open");
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

/// After idle frames populate the keytip cache on Home, switching tabs updates the active tab's
/// keytips so Insert badges run Insert commands and switching back to Home runs Home commands.
#[test]
fn keytips_update_controls_when_switching_tabs() {
    let badge_for = |a: &WordApp, tab: &str, command_id: &str| -> String {
        let reserved = reserved_letters(a);
        let folder = craft::i18n::Folder::new("en");
        let tab_controls = KeytipCache::build_tab_controls(a, tab);
        let labels: Vec<(&str, Option<&str>)> = tab_controls.iter().map(|c| (c.label, c.letters)).collect();
        let badges = craft::keymap::mnemonic::assign_keytips(&labels, &reserved, &folder);
        tab_controls
            .iter()
            .zip(badges)
            .find(|(c, _)| c.command.id == command_id)
            .map(|(_, badge)| badge)
            .expect("command has a keytip badge on its tab")
    };
    let press_keytip_seq = |h: &mut egui_kittest::Harness<'static, WordApp>, seq: &str| {
        for ch in seq.chars() {
            let name = ch.to_ascii_uppercase().to_string();
            let key = egui::Key::from_name(&name).unwrap();
            let text = ch.to_ascii_lowercase().to_string();
            h.input_mut().events.extend([key_event(key, true, egui::Modifiers::NONE), egui::Event::Text(text)]);
            crate::test_harness::frame(h);
        }
    };

    let mut h = harness(app());
    // Let idle frames populate the keytip cache on the initial Home tab.
    crate::test_harness::frames(&mut h, 4);
    assert_eq!(h.state().ui.tab, "Home");

    // Switch to Insert via keytips (F10, N) and fire Page Break through its Insert keytip badge.
    let page_break_badge = badge_for(h.state(), "Insert", "insert.pageBreak");
    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    press_keytip_seq(&mut h, tab_letter("Insert"));
    assert_eq!(h.state().ui.tab, "Insert", "F10 and the Insert tab badge switch to Insert");
    press_keytip_seq(&mut h, &page_break_badge);
    crate::test_harness::frames(&mut h, 3);
    assert!(
        h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains('\u{000C}'),
        "Insert keytip badge {page_break_badge} runs insert.pageBreak after switching from Home"
    );
    assert!(!bold(&mut h), "switching to Insert does not fire Home's Bold command");

    // Switch back to Home via keytips (F10, H) and fire Bold through its Home keytip badge.
    let bold_badge = badge_for(h.state(), "Home", "format.bold");
    h.input_mut().events.push(key_event(egui::Key::F10, true, egui::Modifiers::NONE));
    crate::test_harness::frame(&mut h);
    press_keytip_seq(&mut h, tab_letter("Home"));
    assert_eq!(h.state().ui.tab, "Home", "F10 and the Home tab badge switch back to Home");
    press_keytip_seq(&mut h, &bold_badge);
    crate::test_harness::frames(&mut h, 3);
    assert!(bold(&mut h), "Home keytip badge {bold_badge} runs format.bold after switching back from Insert");
}

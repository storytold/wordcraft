use super::*;

/// Builds an IME edit outcome test fixture with `selection_changed` false.
fn outcome(classified: bool, content: bool, state: bool, transformed: bool) -> ImeEditOutcome {
    ImeEditOutcome { classified, content_changed: content, state_changed: state, transformed, selection_changed: false }
}

/// The central gate: a classified no-op leaves an earlier receipt alone, while any
/// unprovable mutation forces `Other`.
#[test]
fn ime_receipt_gate_distinguishes_noop_from_unproven_mutation() {
    let same = egui::ImeMutationStamp::from_committed("ab", 2, 2, None);
    assert_eq!(classify_ime_proof(outcome(true, false, false, false), &same, &same), ImeProof::NoMutation);
    let changed = egui::ImeMutationStamp::from_committed("abx", 3, 3, None);
    assert_eq!(classify_ime_proof(outcome(true, true, true, false), &same, &changed), ImeProof::Certified);
    assert_eq!(classify_ime_proof(outcome(true, true, true, true), &same, &changed), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(outcome(false, false, true, false), &same, &changed), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(outcome(true, true, true, false), &same, &same), ImeProof::UnprovenMutation);
}

/// Unavailable receipts need both checked revisions and a strict increase; mixed transitions
/// need the unavailable endpoint's revision.
#[test]
fn ime_receipt_gate_checks_unavailable_revisions() {
    let ten = egui::ImeMutationStamp::unavailable_at_revision(10);
    let eleven = egui::ImeMutationStamp::unavailable_at_revision(11);
    let missing = egui::ImeMutationStamp::unavailable();
    let content = outcome(true, true, true, false);
    assert_eq!(classify_ime_proof(content, &ten, &eleven), ImeProof::Certified);
    assert_eq!(classify_ime_proof(content, &eleven, &ten), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(content, &ten, &ten), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(content, &missing, &eleven), ImeProof::UnprovenMutation);
    let available = egui::ImeMutationStamp::from_committed("abx", 3, 3, None);
    assert_eq!(classify_ime_proof(content, &ten, &available), ImeProof::Certified);
    assert_eq!(classify_ime_proof(content, &missing, &available), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(content, &available, &ten), ImeProof::Certified);
    assert_eq!(classify_ime_proof(content, &available, &missing), ImeProof::UnprovenMutation);
}

/// A revision-only unavailable change is a changed observation, not a no-op.
#[test]
fn ime_receipt_gate_treats_revision_only_changes_as_unproven() {
    let ten = egui::ImeMutationStamp::unavailable_at_revision(10);
    let eleven = egui::ImeMutationStamp::unavailable_at_revision(11);
    assert_eq!(classify_ime_proof(outcome(true, false, false, false), &ten, &eleven), ImeProof::UnprovenMutation);
}

/// An exhausted engine allocator emits `Unavailable` without a revision. It can never prove
/// a content change; a classified no-op still leaves an earlier receipt alone.
#[test]
fn ime_receipt_gate_rejects_exhausted_unavailable_stamps() {
    let exhausted = egui::ImeMutationStamp::unavailable();
    assert_eq!(classify_ime_proof(outcome(true, true, true, false), &exhausted, &exhausted), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(outcome(true, false, false, false), &exhausted, &exhausted), ImeProof::NoMutation);
}

/// #1678: Ctrl+F1 collapses or expands the ribbon (not on macOS); plain F1 stays Help's.
#[test]
fn ctrl_f1_collapses_the_ribbon() {
    let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
    let expect = if cfg!(target_os = "macos") { None } else { Some(("ui.collapseRibbon", json!({}))) };
    assert_eq!(ui_shortcut(Key::F1, ctrl), expect);
    assert_eq!(ui_shortcut(Key::F1, Modifiers::NONE), None);
    assert_eq!(ui_shortcut(Key::F1, Modifiers { shift: true, ..ctrl }), None);
}

/// #1678: a Ctrl+F1 the user binds runs their command instead of collapsing the ribbon.
#[test]
fn a_bound_ctrl_f1_beats_the_ribbon_toggle() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.session.keymap = wordcraft_engine::keymap::KeyMap::from_value(&json!({"assigned": {"Mod+F1": "view.focus"}}));
    assert!(dispatch(&mut a, Key::F1, Modifiers { ctrl: true, command: true, ..Default::default() }));
    assert!(a.session.view.focus_mode && !a.ui.ribbon_collapsed);
}

/// #1678 review: only Ctrl+F1 gives way to a binding; a user's Alt+Q binding leaves Alt+Q
/// opening the command search.
#[test]
fn a_bound_alt_q_still_opens_command_search() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.session.keymap = wordcraft_engine::keymap::KeyMap::from_value(&json!({"assigned": {"Alt+Q": "view.focus"}}));
    assert!(dispatch(&mut a, Key::Q, Modifiers { alt: true, ..Default::default() }));
    if cfg!(target_os = "macos") {
        // Option+Q is no interface shortcut on macOS, so the binding runs.
        assert!(a.session.view.focus_mode);
    } else {
        assert!(!a.session.view.focus_mode, "the binding doesn't run");
        assert!(matches!(a.dialog, Some(crate::dialogs::Dialog::Commands { .. })), "Alt+Q opens the command search");
    }
}

#[test]
fn shift_backspace_still_erases() {
    assert_eq!(shift_variant_params("text.backspace"), Some(json!({})));
    assert_eq!(shift_variant_params("caret.left"), Some(json!({"extend": true})));
    assert_eq!(shift_variant_params("text.delete"), None);
}

#[test]
fn alt_q_opens_command_search() {
    let m = |alt, shift, command| egui::Modifiers { alt, shift, command, ..Default::default() };
    let tell_me = Some(("ui.dialog", json!({"name": "commands"})));
    if cfg!(target_os = "macos") {
        assert_eq!(ui_shortcut(Key::Q, m(true, false, false)), None, "Option+Q types a character");
    } else {
        assert_eq!(ui_shortcut(Key::Q, m(true, false, false)), tell_me);
    }
    assert_eq!(ui_shortcut(Key::Q, m(true, true, false)), None, "Alt+Shift+Q");
    assert_eq!(ui_shortcut(Key::Q, m(true, false, true)), None, "Mod+Alt+Q");
    assert_eq!(ui_shortcut(Key::Q, m(false, false, false)), None, "a plain Q");
    assert_eq!(ui_shortcut(Key::W, m(true, false, false)), None);
}

#[test]
fn cmd_option_r_collapses_the_ribbon_on_macos() {
    let m = |mac_cmd, alt, shift| egui::Modifiers { mac_cmd, command: mac_cmd, alt, shift, ..Default::default() };
    let collapse = Some(("ui.collapseRibbon", json!({})));
    assert_eq!(ui_shortcut(Key::R, m(true, true, false)), if cfg!(target_os = "macos") { collapse } else { None });
    assert_eq!(ui_shortcut(Key::R, m(true, false, false)), None, "Mod+R aligns right");
    assert_eq!(ui_shortcut(Key::R, m(false, true, false)), None, "Option+R types a character");
    assert_eq!(ui_shortcut(Key::R, m(true, true, true)), None, "Mod+Alt+Shift+R");
}

#[test]
fn only_plain_paste_releases_paste_a_clipboard_picture() {
    let m = |command, shift, alt| egui::Modifiers { command, shift, alt, ..Default::default() };
    assert!(is_paste_release(Key::V, m(true, false, false)), "Mod+V");
    assert!(is_paste_release(Key::Paste, m(false, false, false)), "the Paste key");
    assert!(!is_paste_release(Key::V, m(true, true, false)), "Mod+Shift+V is Paste Formatting");
    assert!(!is_paste_release(Key::V, m(true, false, true)), "Mod+Alt+V is Paste Special");
    assert!(!is_paste_release(Key::V, m(false, false, false)), "a plain V");
    assert!(is_paste_special_release(Key::V, m(true, false, true)), "Mod+Alt+V opens Paste Special");
    assert!(!is_paste_special_release(Key::V, m(true, true, true)) && !is_paste_special_release(Key::V, m(true, false, false)));
}

/// An empty document whose clipboard holds a 4 × 4 picture.
fn picture_app() -> WordApp {
    use crate::paste_picture::ClipboardPicture;
    let services = crate::Services {
        clipboard_picture: Some(Box::new(|| Some(ClipboardPicture::Pixels { width: 4, height: 4, rgba: vec![200; 4 * 4 * 4] }))),
        ..Default::default()
    };
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services)
}

/// How many pictures the document holds (the Selection Pane's list).
fn pictures(a: &mut WordApp) -> usize {
    a.session.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(|x| x.len()).unwrap_or(0)
}

/// One canvas frame with `modifiers` held, then `events`.
fn mods_frame(ctx: &egui::Context, a: &mut WordApp, modifiers: Modifiers, events: Vec<egui::Event>) {
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| canvas_events(a, ui.ctx())).drop_without_applying_deltas();
}

/// A press or release of `key` with `modifiers`.
fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
}

/// Left Ctrl+V with one hand often lets Ctrl up a moment before V. egui-winit keeps the
/// Mod+V press, so the canvas sees only Ctrl's release, then V's without Mod: still a paste.
#[test]
fn mod_v_released_mod_first_pastes_the_clipboard_picture() {
    let ctx = egui::Context::default();
    let mut a = picture_app();
    let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
    mods_frame(&ctx, &mut a, ctrl, vec![key_event(Key::ControlLeft, true, ctrl)]);
    // The Mod+V press: egui-winit sends no event for it.
    mods_frame(&ctx, &mut a, ctrl, vec![]);
    mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::ControlLeft, false, Modifiers::NONE)]);
    mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
    assert_eq!(pictures(&mut a), 1);

    // A plain V afterwards (its press arrives) isn't a paste.
    mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::V, true, Modifiers::NONE)]);
    mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
    assert_eq!(pictures(&mut a), 1);

    // Nor is Ctrl+B, Ctrl up, then a V release whose press never came (focus moved in).
    mods_frame(&ctx, &mut a, ctrl, vec![key_event(Key::B, true, ctrl)]);
    mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::ControlLeft, false, Modifiers::NONE), egui::Event::WindowFocused(true)]);
    mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
    assert_eq!(pictures(&mut a), 1);

    // Mod+Shift+V released Mod first stays Paste Formatting, not a picture paste.
    let ctrl_shift = Modifiers { shift: true, ..ctrl };
    mods_frame(&ctx, &mut a, ctrl_shift, vec![]);
    mods_frame(&ctx, &mut a, Modifiers::SHIFT, vec![key_event(Key::ControlLeft, false, Modifiers::SHIFT)]);
    mods_frame(&ctx, &mut a, Modifiers::SHIFT, vec![key_event(Key::V, false, Modifiers::SHIFT)]);
    assert_eq!(pictures(&mut a), 1);

    // Released V first, as before, still pastes.
    let mut a = picture_app();
    mods_frame(&ctx, &mut a, ctrl, vec![]);
    mods_frame(&ctx, &mut a, ctrl, vec![key_event(Key::V, false, ctrl)]);
    assert_eq!(pictures(&mut a), 1);
}

/// Ctrl down, the kept Mod+V press, Ctrl up and V up can all arrive in one frame: the frame's
/// modifiers no longer have Mod, but the ModifiersChanged events in between did.
#[test]
fn mod_v_within_one_frame_pastes_the_clipboard_picture() {
    let ctx = egui::Context::default();
    let mut a = picture_app();
    let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
    let events = vec![
        egui::Event::ModifiersChanged(ctrl),
        key_event(Key::ControlLeft, true, ctrl),
        egui::Event::ModifiersChanged(Modifiers::NONE),
        key_event(Key::ControlLeft, false, Modifiers::NONE),
        key_event(Key::V, false, Modifiers::NONE),
    ];
    ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| canvas_events(&mut a, ui.ctx())).drop_without_applying_deltas();
    assert_eq!(pictures(&mut a), 1);
}

/// egui-winit 0.37 keeps both the Mod+V press and its release, and sends `PasteImage` when the
/// clipboard holds a picture and no text (Windows, macOS, X11): that pastes the picture, once.
#[test]
fn paste_image_event_pastes_the_clipboard_picture() {
    let ctx = egui::Context::default();
    let mut a = picture_app();
    let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
    let image = egui::Event::PasteImage(std::sync::Arc::new(egui::ColorImage::filled([4, 4], egui::Color32::WHITE)));
    mods_frame(&ctx, &mut a, ctrl, vec![key_event(Key::ControlLeft, true, ctrl), image]);
    mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::ControlLeft, false, Modifiers::NONE)]);
    assert_eq!(pictures(&mut a), 1);
}

/// A V release keeps its own modifiers unless Mod went up first and no V press arrived.
#[test]
fn paste_release_modifiers_fall_back_to_those_held_with_mod() {
    let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
    assert_eq!(paste_release_modifiers(ctrl, false, None), ctrl, "Mod still down");
    assert_eq!(paste_release_modifiers(Modifiers::NONE, false, Some(ctrl)), ctrl, "Mod up first, press kept by egui-winit");
    assert_eq!(paste_release_modifiers(Modifiers::NONE, true, Some(ctrl)), Modifiers::NONE, "a plain V's press arrived");
    assert_eq!(paste_release_modifiers(Modifiers::NONE, false, None), Modifiers::NONE);
}

#[test]
fn shift_delete_is_only_shift_and_delete() {
    let m = |shift, ctrl, alt| egui::Modifiers { shift, ctrl, command: ctrl, alt, ..Default::default() };
    assert!(is_shift_delete(Key::Delete, m(true, false, false)));
    assert!(!is_shift_delete(Key::Delete, m(false, false, false)), "Delete alone erases");
    assert!(!is_shift_delete(Key::Delete, m(true, true, false)), "Ctrl+Shift+Delete");
    assert!(!is_shift_delete(Key::Delete, m(true, false, true)), "Alt+Shift+Delete");
    assert!(!is_shift_delete(Key::Backspace, m(true, false, false)), "Shift+Backspace erases (#707)");
}

#[test]
fn shift_f10_is_the_context_menu_key() {
    let m = |shift, ctrl, alt| egui::Modifiers { shift, ctrl, command: ctrl, alt, ..Default::default() };
    assert!(is_context_menu_key(Key::F10, m(true, false, false)));
    assert!(!is_context_menu_key(Key::F10, m(false, false, false)), "F10 alone shows keytips");
    assert!(!is_context_menu_key(Key::F10, m(true, true, false)), "Ctrl+Shift+F10");
    assert!(!is_context_menu_key(Key::F9, m(true, false, false)), "Shift+F9 is a field key");
}

#[test]
fn alt_shortcuts_work_without_the_canvas_focused() {
    let m = |command, alt, shift| egui::Modifiers { command, alt, shift, ..Default::default() };
    assert!(is_global_shortcut(Key::Q, m(false, true, false)), "Alt+Q (#819)");
    assert!(is_global_shortcut(Key::D, m(false, true, true)), "Alt+Shift+D");
    assert!(is_global_shortcut(Key::B, m(true, false, false)), "Mod+B");
    assert!(is_global_shortcut(Key::F7, m(false, false, false)), "F7");
    assert!(!is_global_shortcut(Key::Q, m(false, false, false)), "a plain letter is typing");
    assert!(!is_global_shortcut(Key::Q, m(false, false, true)), "Shift+letter is typing");
}

/// Alt held, keypad 0 2 3 3, Alt released: é (й on a Russian system) is typed, the digits
/// aren't, and keytips stay off (#961). Linux sends each digit's text too; Windows may send the
/// composed character.
#[test]
fn alt_numpad_code_types_its_character_without_keytips() {
    if cfg!(target_os = "macos") {
        return;
    }
    let doc_text = |a: &WordApp| a.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    let ctx = egui::Context::default();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    // Windows reads codes in the system locale's code pages (0233 is é in 1252 and 1250, й in
    // 1251); elsewhere the ANSI page is 1252 and the OEM page the locale's own. The app records
    // the locale when it starts, so ask after `WordApp::new`.
    let (ansi, oem_page) = if cfg!(target_os = "windows") {
        wordcraft_engine::cmd::text::system_code_pages()
    } else {
        (1252, wordcraft_engine::cmd::text::system_oem_code_page())
    };
    let typed = wordcraft_engine::cmd::text::alt_code_char("0233", ansi, oem_page).unwrap_or('?');
    let alt = Modifiers { alt: true, ..Default::default() };
    let key = |key, pressed, modifiers| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers };
    let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            crate::keytips::logic(a, ui.ctx());
            canvas_events(a, ui.ctx());
        })
        .drop_without_applying_deltas();
    };
    frame(&mut a, vec![egui::Event::ModifiersChanged(alt), key(Key::AltLeft, true, alt)]);
    for (k, t) in [(Key::Num0, "0"), (Key::Num2, "2"), (Key::Num3, "3"), (Key::Num3, "3")] {
        frame(&mut a, vec![key(k, true, alt), egui::Event::Text(t.into()), key(k, false, alt)]);
    }
    assert_eq!(doc_text(&a).trim_end(), "", "nothing typed while Alt is held");
    frame(&mut a, vec![key(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(doc_text(&a).trim_end(), typed.to_string());
    assert_eq!(a.ui.keytips, crate::keytips::Phase::Off, "an Alt code doesn't open keytips");
    // The platform's own composed character isn't typed twice; the next typing is.
    frame(&mut a, vec![egui::Event::Text(typed.to_string())]);
    frame(&mut a, vec![key(Key::E, true, Modifiers::NONE), egui::Event::Text("é".into())]);
    assert_eq!(doc_text(&a).trim_end(), format!("{typed}é"));
    // Without the leading zero: the system locale's OEM code page (233 is Θ in 437, Ú in 850).
    frame(&mut a, vec![egui::Event::ModifiersChanged(alt), key(Key::AltLeft, true, alt)]);
    for k in [Key::Num2, Key::Num3, Key::Num3] {
        frame(&mut a, vec![key(k, true, alt), key(k, false, alt)]);
    }
    frame(&mut a, vec![key(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
    let oem = wordcraft_engine::cmd::text::alt_code_char("233", ansi, oem_page);
    assert_eq!(doc_text(&a).trim_end(), format!("{typed}é{}", oem.unwrap_or('?')));
    // A bare Alt tap still opens keytips.
    frame(&mut a, vec![egui::Event::ModifiersChanged(alt), key(Key::AltLeft, true, alt)]);
    frame(&mut a, vec![key(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(a.ui.keytips, crate::keytips::Phase::Tabs);
}

/// #2388: AltGr (Right Alt on a Polish layout under Wayland) arrives as the Right Alt key
/// followed by a modifiers event without Alt (it's Level 3 Shift). That event read as Alt
/// let go opened keytips while AltGr was still held, and the next letter went to them
/// instead of typing ś, ł, ż. A bare AltGr tap doesn't open keytips either; a Right Alt
/// that is Alt (US layout) still does.
#[test]
fn altgr_types_its_characters_without_keytips() {
    if cfg!(target_os = "macos") {
        return;
    }
    let doc_text = |a: &WordApp| a.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    let ctx = egui::Context::default();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let none = Modifiers::NONE;
    let key = |key, pressed, modifiers| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers };
    let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            crate::keytips::logic(a, ui.ctx());
            canvas_events(a, ui.ctx());
        })
        .drop_without_applying_deltas();
    };
    let altgr_down = || vec![key(Key::AltRight, true, none), egui::Event::ModifiersChanged(none)];
    let altgr_up = || vec![key(Key::AltRight, false, none), egui::Event::ModifiersChanged(none)];
    // Held through three letters, one frame per event batch.
    frame(&mut a, altgr_down());
    assert_eq!(a.ui.keytips, crate::keytips::Phase::Off, "AltGr down doesn't open keytips");
    for (k, t) in [(Key::S, "ś"), (Key::L, "ł"), (Key::Z, "ż")] {
        frame(&mut a, vec![key(k, true, none), egui::Event::Text(t.into())]);
        frame(&mut a, vec![key(k, false, none)]);
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Off, "{t}");
    }
    frame(&mut a, altgr_up());
    assert_eq!(doc_text(&a).trim_end(), "śłż");
    assert_eq!(a.ui.keytips, crate::keytips::Phase::Off);
    // All in one frame.
    frame(&mut a, [altgr_down(), vec![key(Key::A, true, none), egui::Event::Text("ą".into()), key(Key::A, false, none)], altgr_up()].concat());
    assert_eq!(doc_text(&a).trim_end(), "śłżą");
    assert_eq!(a.ui.keytips, crate::keytips::Phase::Off);
    // A bare AltGr tap, in two frames or one.
    frame(&mut a, altgr_down());
    frame(&mut a, altgr_up());
    assert_eq!(a.ui.keytips, crate::keytips::Phase::Off, "an AltGr tap isn't an Alt tap");
    frame(&mut a, [altgr_down(), altgr_up()].concat());
    assert_eq!(a.ui.keytips, crate::keytips::Phase::Off);
    // Right Alt that is Alt still opens keytips.
    let alt = Modifiers { alt: true, ..Default::default() };
    frame(&mut a, vec![key(Key::AltRight, true, none), egui::Event::ModifiersChanged(alt)]);
    frame(&mut a, vec![key(Key::AltRight, false, alt), egui::Event::ModifiersChanged(none)]);
    assert_eq!(a.ui.keytips, crate::keytips::Phase::Tabs);
}

fn alt_frame_app() -> (egui::Context, WordApp) {
    let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    (egui::Context::default(), a)
}

fn alt_frame(ctx: &egui::Context, a: &mut WordApp, events: Vec<egui::Event>) {
    ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| canvas_events(a, ui.ctx())).drop_without_applying_deltas();
}

fn alt_key(key: Key, pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key { key, physical_key: Some(key), pressed, repeat, modifiers: Modifiers { alt: true, ..Default::default() } }
}

/// Option+Shift+- types an em dash outside Outline view, where Word's Alt+Shift+- (Collapse)
/// can't run, so its shortcut doesn't swallow the character (#1933).
#[test]
fn disabled_alt_shortcut_leaves_the_typed_character() {
    let (ctx, mut a) = alt_frame_app();
    let alt_shift = Modifiers { alt: true, shift: true, ..Default::default() };
    let key = |pressed| egui::Event::Key { key: Key::Minus, physical_key: Some(Key::Minus), pressed, repeat: false, modifiers: alt_shift };
    alt_frame(&ctx, &mut a, vec![key(true), egui::Event::Text("\u{2014}".into()), key(false)]);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "\u{2014}");
}

/// Holding a digit with Alt adds it once: its auto-repeat presses don't (#961).
#[test]
fn alt_code_ignores_auto_repeat() {
    if cfg!(target_os = "macos") {
        return;
    }
    let (ctx, mut a) = alt_frame_app();
    alt_frame(&ctx, &mut a, vec![alt_key(Key::Num6, true, false), alt_key(Key::Num6, true, true), alt_key(Key::Num6, true, true)]);
    alt_frame(&ctx, &mut a, vec![alt_key(Key::Num5, true, false)]);
    assert_eq!(a.canvas.alt_code, "65");
    alt_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "A");
}

/// Switching to another window mid-code drops the digits: releasing Alt later types nothing.
#[test]
fn alt_code_is_dropped_when_the_window_loses_focus() {
    if cfg!(target_os = "macos") {
        return;
    }
    let (ctx, mut a) = alt_frame_app();
    alt_frame(&ctx, &mut a, vec![alt_key(Key::Num6, true, false), alt_key(Key::Num5, true, false)]);
    alt_frame(&ctx, &mut a, vec![egui::Event::WindowFocused(false)]);
    assert_eq!(a.canvas.alt_code, "");
    alt_frame(&ctx, &mut a, vec![egui::Event::WindowFocused(true), egui::Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "");
}

/// Windows composes Alt codes with the system's code pages: for a code up to 255 its
/// character replaces ours (Polish Alt+0185 is ą, not ¹); above 255 Windows reduces the code
/// to a byte, so ours stays and its text isn't typed (#961).
#[test]
fn system_composed_alt_code_character_wins_up_to_255() {
    let (ctx, mut a) = alt_frame_app();
    let text = |a: &WordApp| a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end().to_string();
    let _ = a.run("text.typeCode", json!({"code": "0185", "ansi": 1252}));
    a.canvas.alt_code_echo = Some(AltCodeEcho { typed: "¹".into(), system_composes: true, byte_code: true });
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("ą".into())]);
    assert_eq!(text(&a), "ą");
    let _ = a.run("text.typeCode", json!({"code": "8364"}));
    a.canvas.alt_code_echo = Some(AltCodeEcho { typed: "€".into(), system_composes: true, byte_code: false });
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("|".into())]);
    assert_eq!(text(&a), "ą€");
    // Elsewhere a different text after the code is ordinary typing.
    a.canvas.alt_code_echo = Some(AltCodeEcho { typed: "€".into(), system_composes: false, byte_code: false });
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("x".into())]);
    assert_eq!(text(&a), "ą€x");
}

fn body(a: &WordApp) -> String {
    a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end_matches(['\r', '\n']).to_string()
}

fn surround(before_chars: usize, after_chars: usize) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, after_chars })
}

/// A real body replacement invalidates the old preedit even if its text is unchanged.
/// Its queued replacement must not edit the new body or add an Undo checkpoint.
#[test]
fn ime_interrupted_body_preedit_drops_queued_replacement_before_mutation() {
    for replacement in [
        vec![egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))],
        vec![surround(1, 1), egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))],
        vec![surround_bytes(1, 1), egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))],
        vec![surround_bytes(1, 1), egui::Event::Text("stale".into()), egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))],
    ] {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "ABCDE"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None })]);
        assert_eq!(a.canvas.ime_preedit, "に");
        a.run("document.setText", json!({"text": "ABCDE"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
        let depth = a.session.undo_depth();
        alt_frame(&ctx, &mut a, replacement);
        assert_eq!(body(&a), "ABCDE", "queued events cannot replace text in the new body");
        assert_eq!(a.session.sel.focus.off, 2, "not even the new caret moves");
        assert_eq!(a.session.undo_depth(), depth, "rejection happens before any transaction");
        assert!(a.canvas.ime_preedit.is_empty());
        assert!(a.canvas.ime_interrupt_now);
    }
}

/// Exiting the equation through a real key in the same raw batch invalidates its preedit
/// before the following body deletion/commit, as does an external target change.
#[test]
fn ime_interrupted_equation_preedit_drops_queued_body_replacement() {
    for in_batch in [false, true] {
        let (ctx, mut a) = alt_frame_app();
        a.run("insert.equation", json!({"linear": "abc"})).unwrap();
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None })]);
        assert_eq!(a.canvas.ime_preedit, "に");
        let mut events = Vec::new();
        if in_batch {
            events.push(egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        } else {
            a.run("equation.exit", json!({})).unwrap();
        }
        let before = body(&a);
        let depth = a.session.undo_depth();
        events.extend([surround(1, 0), egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))]);
        alt_frame(&ctx, &mut a, events);
        assert!(a.session.math.is_none(), "the equation exit itself still takes effect");
        assert_eq!(body(&a), before, "the equation's queued replacement cannot edit the body");
        assert_eq!(a.session.undo_depth(), depth, "neither rejected event creates an Undo step");
    }
}

/// Untagged commits are legitimate without an interrupted composition, after a fresh
/// preedit, after the one stale replacement, and after an empty intervening batch.
#[test]
fn ime_interrupted_preedit_does_not_suppress_fresh_input() {
    for fresh_case in 0..4 {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "ABCDE"})).unwrap();
        if fresh_case != 0 {
            alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "old".into(), active_range_chars: None })]);
        }
        a.run("document.setText", json!({"text": "ABCDE"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
        let depth = a.session.undo_depth();
        let mut events = Vec::new();
        match fresh_case {
            1 => events.push(egui::Event::Ime(egui::ImeEvent::Preedit { text: "fresh".into(), active_range_chars: None })),
            2 => events.push(egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))),
            3 => alt_frame(&ctx, &mut a, vec![]),
            _ => {}
        }
        events.push(egui::Event::Ime(egui::ImeEvent::Commit("X".into())));
        alt_frame(&ctx, &mut a, events);
        assert_eq!(body(&a), "ABXCDE", "fresh case {fresh_case} must remain eligible");
        assert_eq!(a.session.undo_depth(), depth + 1);
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "ABCDE");
    }
}

/// The web's text agent keeps what it typed and, when its value changes other than by an
/// addition at the end, sends `DeleteSurrounding` for the changed tail and then the new text
/// (#1480). The document follows the agent's value instead of repeating it.
#[test]
fn delete_surrounding_from_the_web_text_agent_does_not_repeat_text() {
    let (ctx, mut a) = alt_frame_app();
    // The trace from #1480: the agent's caret jumped to 0 after `м`, so it reported `иabм`
    // and `риabм` as its whole new tail.
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("a".into())]);
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("b".into())]);
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("м".into())]);
    alt_frame(&ctx, &mut a, vec![surround(3, 0), egui::Event::Text("иabм".into())]);
    alt_frame(&ctx, &mut a, vec![surround(4, 0), egui::Event::Text("риabм".into())]);
    assert_eq!(body(&a), "риabм");
    // A correction (`teh` -> `the`): erase three characters, type the replacement.
    alt_frame(&ctx, &mut a, vec![egui::Event::Text(" teh".into())]);
    alt_frame(&ctx, &mut a, vec![surround(3, 0), egui::Event::Text("the".into())]);
    assert_eq!(body(&a), "риabм the");
    // A plain deletion (Android Backspace): no text follows, or an empty one.
    alt_frame(&ctx, &mut a, vec![surround(1, 0), egui::Event::Text(String::new())]);
    assert_eq!(body(&a), "риabм th");
    alt_frame(&ctx, &mut a, vec![surround(2, 0)]);
    assert_eq!(body(&a), "риabм ");
}

/// Builds an `ImeEvent::DeleteSurroundingBytes` event with the given before and after byte counts.
fn surround_bytes(before_bytes: usize, after_bytes: usize) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes })
}

/// macOS and Wayland IME replacement counts UTF-8 bytes and must delete exactly that range.
/// A byte count into the middle of a character is refused rather than slicing the string.
#[test]
fn delete_surrounding_bytes_replaces_exact_utf8_and_joins_one_undo_step() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "aé👍b"})).unwrap();
    // The caret is before `b`, after the four-byte emoji.
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 7));
    alt_frame(&ctx, &mut a, vec![surround_bytes(4, 0), egui::Event::Text("x".into())]);
    assert_eq!(body(&a), "aéxb");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "aé👍b", "delete plus commit is one undo step");

    // A range that starts inside the emoji is not a UTF-8 boundary: nothing changes.
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 7));
    alt_frame(&ctx, &mut a, vec![surround_bytes(3, 0)]);
    assert_eq!(body(&a), "aé👍b");
    // Beyond the paragraph is refused too, and never joins the next paragraph.
    alt_frame(&ctx, &mut a, vec![surround_bytes(0, 99)]);
    assert_eq!(body(&a), "aé👍b");
    let depth = a.session.undo_depth();
    for (before, after) in [(99, 0), (usize::MAX, 0), (0, usize::MAX)] {
        alt_frame(&ctx, &mut a, vec![surround_bytes(before, after)]);
        assert_eq!(body(&a), "aé👍b", "out-of-range counts must not erase a prefix");
        assert_eq!(a.session.undo_depth(), depth);
    }
}

/// A combining accent is its own UTF-8 range; replacing it does not erase the base letter.
/// `document.setText` leaves the caret at the start, so the test moves it past `e` + accent.
#[test]
fn delete_surrounding_bytes_replaces_a_combining_mark_and_accepts_empty_commit() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "e\u{301}"})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, "e\u{301}".len()));
    alt_frame(&ctx, &mut a, vec![surround_bytes(2, 0), egui::Event::Text(String::new())]);
    assert_eq!(body(&a), "e");
    // A deletion-only batch and a 0/0 batch stay harmless.
    alt_frame(&ctx, &mut a, vec![surround_bytes(1, 0)]);
    assert_eq!(body(&a), "");
    alt_frame(&ctx, &mut a, vec![surround_bytes(0, 0)]);
    assert_eq!(body(&a), "");
}

/// The real macOS/Wayland path is `ImeEvent::Commit`, not `Event::Text`: the byte deletion
/// and the committed replacement are one edit, and Undo restores the original text.
#[test]
fn ime_commit_replaces_delete_surrounding_bytes_in_one_undo_step() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "e\u{301}"})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, "e\u{301}".len()));
    alt_frame(&ctx, &mut a, vec![surround_bytes("e\u{301}".len(), 0), egui::Event::Ime(egui::ImeEvent::Commit("é".into()))]);
    assert_eq!(body(&a), "é", "the combining sequence is replaced by the IME's committed text");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "e\u{301}", "delete plus Commit is one undo step");
}

/// A platform may deliver the `Commit` that replaces a `DeleteSurrounding` in a later raw
/// batch than the deletion. The undo join belongs to the deletion transaction, not to the
/// frame: the commit still shares its checkpoint, so one Undo restores the original input.
#[test]
fn ime_commit_in_a_later_frame_joins_the_delete_surrounding_undo_step() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "teh"})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 3));
    let depth = a.session.undo_depth();

    alt_frame(&ctx, &mut a, vec![surround_bytes(3, 0)]);
    assert_eq!(body(&a), "");
    assert_eq!(a.session.undo_depth(), depth + 1, "the deletion is one step");

    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("the".into()))]);
    assert_eq!(body(&a), "the", "the later commit replaces the erased text");
    assert_eq!(a.session.undo_depth(), depth + 1, "the later commit joins the deletion's checkpoint");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "teh", "one Undo restores the original input");
}

/// An accepted IME commit can also be empty: it finishes composition without adding text.
#[test]
fn ime_empty_commit_finishes_without_inserting() {
    let (ctx, mut a) = alt_frame_app();
    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))]);
    assert_eq!(body(&a), "");
}

/// `after_chars` erases after the caret; both counts are characters, not bytes.
#[test]
fn delete_surrounding_erases_characters_on_both_sides_of_the_caret() {
    let (ctx, mut a) = alt_frame_app();
    a.run("text.insert", json!({"text": "мир дом"})).unwrap();
    for _ in 0..3 {
        a.run("caret.left", json!({})).unwrap();
    }
    // The caret is after `мир `: two before (`р `) and one after (`д`).
    alt_frame(&ctx, &mut a, vec![surround(2, 1)]);
    assert_eq!(body(&a), "миом");
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("X".into())]);
    assert_eq!(body(&a), "миXом", "the caret stays where the deletion was");
}

/// The counts come from outside: larger than the paragraph they stop at its ends, never
/// joining paragraphs or panicking.
#[test]
fn delete_surrounding_stays_inside_the_paragraph() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "one\ntwo\nthree"})).unwrap();
    a.run("caret.docStart", json!({})).unwrap();
    // After the `t` of `two`.
    for _ in 0..5 {
        a.run("caret.right", json!({})).unwrap();
    }
    alt_frame(&ctx, &mut a, vec![surround(usize::MAX, usize::MAX)]);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one\n\nthree");
    alt_frame(&ctx, &mut a, vec![surround(5, 5), egui::Event::Text("2".into())]);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one\n2\nthree");
}

/// The agent's rewrite belongs to the typing it corrects: one Undo takes back the typing
/// and its correction together, not a half-erased word.
#[test]
fn delete_surrounding_joins_the_typing_undo_step() {
    let (ctx, mut a) = alt_frame_app();
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("teh".into())]);
    alt_frame(&ctx, &mut a, vec![surround(3, 0), egui::Event::Text("the".into())]);
    assert_eq!(body(&a), "the");
    // Typing goes on in the same group, not one step per keystroke.
    alt_frame(&ctx, &mut a, vec![egui::Event::Text(" more".into())]);
    alt_frame(&ctx, &mut a, vec![surround(4, 0), egui::Event::Text("mores".into())]);
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("t".into())]);
    assert_eq!(body(&a), "the morest");
    assert_eq!(a.session.undo_label(), Some("Typing"));
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "");
}

/// The rewritten text keeps the formatting of the text it replaces: a bold `helo` corrected
/// to `hello` stays bold, as typing over a word does.
#[test]
fn delete_surrounding_keeps_the_replaced_texts_formatting() {
    let (ctx, mut a) = alt_frame_app();
    a.run("text.insert", json!({"text": "plain "})).unwrap();
    a.run("format.bold", json!({})).unwrap();
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("helo".into())]);
    alt_frame(&ctx, &mut a, vec![surround(4, 0), egui::Event::Text("hello".into())]);
    assert_eq!(body(&a), "plain hello");
    let f = a.session.sel.focus.clone();
    let para = a.session.doc.para_at(&f).expect("a paragraph");
    let bold: Vec<bool> = para.text.char_indices().map(|(i, _)| para.props_of_char(i).bold == Some(true)).collect();
    assert_eq!(bold, [false, false, false, false, false, false, true, true, true, true, true], "{bold:?}");
}

/// Overtype replaces characters after the caret when typing, but the text an input method
/// sends to replace what it erased only replaces that.
#[test]
fn delete_surrounding_text_does_not_overtype() {
    let (ctx, mut a) = alt_frame_app();
    a.run("text.insert", json!({"text": "texXY"})).unwrap();
    for _ in 0..2 {
        a.run("caret.left", json!({})).unwrap();
    }
    a.run("file.options", json!({"overtype": true})).unwrap();
    alt_frame(&ctx, &mut a, vec![surround(3, 0), egui::Event::Text("Texas".into())]);
    assert_eq!(body(&a), "TexasXY");
}

/// A key pressed between the erasure and the text ends the input method's rewrite: the text
/// is ordinary typing, its own undo step.
#[test]
fn delete_surrounding_rewrite_ends_at_a_key_press() {
    let (ctx, mut a) = alt_frame_app();
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("ab".into())]);
    let key = egui::Event::Key { key: Key::F20, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    alt_frame(&ctx, &mut a, vec![surround(1, 0), key, egui::Event::Text("X".into())]);
    assert_eq!(body(&a), "aX");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "a");
}

/// An automatic change while typing (AutoCorrect `(c)` -> `©`) changes the text behind the
/// web's text agent, so its later counts would be wrong: the page asks it to start afresh.
#[test]
fn automatic_change_asks_the_input_method_to_start_afresh() {
    let (ctx, mut a) = alt_frame_app();
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("Word".into())]);
    alt_frame(&ctx, &mut a, vec![egui::Event::Text(" ".into())]);
    assert!(!a.canvas.ime_reset, "plain typing keeps the agent's text");
    alt_frame(&ctx, &mut a, vec![egui::Event::Text("(c)".into())]);
    alt_frame(&ctx, &mut a, vec![egui::Event::Text(" ".into())]);
    assert_eq!(body(&a), "Word \u{a9} ");
    assert!(a.canvas.ime_reset, "AutoCorrect changed the text");
}

/// Both sides of one `DeleteSurrounding` are one request: when the second `text.backspace`
/// is refused (here by a content control locked against editing), the first half is rolled
/// back and no rewrite or receipt is promised.
#[test]
fn a_refused_second_delete_surrounding_rolls_the_request_back() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "ab"})).unwrap();
    // Wrap `b` in a rich-text control that refuses edits, leaving `a` editable.
    a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 2) };
    a.run("developer.richText", json!({"tag": "locked"})).unwrap();
    a.run("developer.properties", json!({"control": "locked", "lockEdit": true})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1));
    let depth = a.session.undo_depth();
    let pending = wordcraft_doc::props::CharProps { bold: Some(true), ..Default::default() };
    a.session.pending = Some(pending.clone());
    a.session.goal_x = Some(123.0);
    alt_frame(&ctx, &mut a, vec![surround(1, 1)]);
    assert_eq!(body(&a), "ab", "the applied left side is rolled back with the refused right one");
    assert_eq!(a.session.undo_depth(), depth, "no half-erasure is left in Undo");
    assert!(a.canvas.ime_join_text.is_none(), "no text rewrite is promised");
    assert!(a.canvas.ime_interrupt_now, "the platform's stale surrounding snapshot is discarded");
    assert_eq!(a.session.sel.focus.off, 1, "the caret is back where the request found it");
    assert_eq!(a.session.pending, Some(pending), "pending formatting is restored");
    assert_eq!(a.session.goal_x, Some(123.0), "the Up/Down goal column is restored");
}

/// A successful body `DeleteSurrounding` followed in the same raw batch by a commit refused
/// by the locked content at the caret must leave no trace but the deletion: the replacement's
/// pending formatting is rolled back, the composition is interrupted, no undo step or
/// fabricated AutoFormat change is recorded, and the caret stays at the deletion.
///
/// Forms mode makes the `za` control fillable and everything outside it protected. Deleting
/// its final `a` is allowed; the following commit starts at the locked `X` control's content
/// start, which typing moves beside the control (as Word does at a content-control edge) and
/// into protected text, where Restrict Editing refuses it. The deletion therefore stands and
/// the replacement does not.
#[test]
fn a_refused_ime_commit_after_a_successful_delete_restores_pending_and_interrupts() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "zaXY"})).unwrap();
    a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 0), focus: wordcraft_doc::Pos::body(0, 2) };
    a.run("developer.richText", json!({"tag": "fill"})).unwrap();
    a.run("select.text", json!({"text": "X"})).unwrap();
    a.run("developer.richText", json!({"tag": "locked"})).unwrap();
    a.run("developer.properties", json!({"control": "locked", "lockEdit": true})).unwrap();
    a.run("review.restrict", json!({"mode": "forms"})).unwrap();
    let locked =
        a.session.doc.control_ranges(wordcraft_doc::StoryRef::Body).into_iter().find(|r| r.control.tag == "locked").expect("the locked control");
    a.session.sel = wordcraft_engine::Selection::caret(locked.content_start());
    a.session.pending = None;
    a.canvas.ime_reset = false;
    let _ = a.session.take_pending_ime_application_change();
    let depth = a.session.undo_depth();
    alt_frame(&ctx, &mut a, vec![surround(1, 0), egui::Event::Ime(egui::ImeEvent::Commit("\"".into()))]);
    assert_eq!(body(&a), "zXY", "the locked replacement is refused, the earlier deletion stands");
    assert_eq!(a.session.undo_depth(), depth + 1, "only the successful deletion added an undo step");
    let locked = a
        .session
        .doc
        .control_ranges(wordcraft_doc::StoryRef::Body)
        .into_iter()
        .find(|r| r.control.tag == "locked")
        .expect("the locked control remains");
    assert_eq!(a.session.sel.focus, locked.content_start(), "the caret stays where the deletion left it");
    assert_eq!(a.session.sel.anchor, locked.content_start());
    assert_eq!(a.session.pending, None, "the refused replacement leaves no pending formatting from the erased text");
    assert!(a.canvas.ime_join_text.is_none(), "the refused replacement promises no rewrite");
    assert!(a.canvas.ime_interrupt_now, "the platform's stale surrounding snapshot is interrupted");
    assert!(!a.canvas.ime_reset, "a refused commit is not an AutoFormat change");
    assert!(!a.session.take_pending_ime_application_change(), "no application-origin receipt is fabricated");
}

/// Track Changes can remove the author's own insertion from the stored text. The right-hand
/// range and the final caret then move left by what was actually removed: the second side
/// must erase the character the platform named, not the one that took its stored offset.
#[test]
fn tracked_delete_surrounding_shifts_past_its_own_insertion() {
    let (ctx, mut a) = alt_frame_app();
    a.run("review.trackChanges", json!({"value": true})).unwrap();
    a.run("text.insert", json!({"text": "abXY"})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
    alt_frame(&ctx, &mut a, vec![surround(1, 1)]);
    assert_eq!(body(&a), "aY", "the own insertion `b` and the following `X` are erased");
    assert_eq!(a.session.sel.focus.off, 1, "the caret points after the stored deletion");
    assert_eq!(a.session.sel.anchor.off, 1);
}

/// Track Changes plus a refusal: the first side really removes this author's own insertion
/// from the stored paragraph, then a locked control refuses the second side. The whole
/// request is rolled back, including the shifted caret and undo stack.
#[test]
fn tracked_own_insertion_then_a_refused_side_rolls_the_whole_request_back() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "aXY"})).unwrap();
    // Lock the following `X`; inserting `b` before it under Track Changes is this author's own
    // insertion, which a tracked delete removes from the stored text instead of marking.
    a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 2) };
    a.run("developer.richText", json!({"tag": "locked"})).unwrap();
    a.run("developer.properties", json!({"control": "locked", "lockEdit": true})).unwrap();
    a.run("review.trackChanges", json!({"value": true})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1));
    a.run("text.insert", json!({"text": "b"})).unwrap();
    assert_eq!(body(&a), "abXY");
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
    let depth = a.session.undo_depth();
    let revisions = a.session.doc.revisions.len();
    let pending = wordcraft_doc::props::CharProps { bold: Some(true), ..Default::default() };
    a.session.pending = Some(pending.clone());
    a.session.goal_x = Some(123.0);
    alt_frame(&ctx, &mut a, vec![surround(1, 1)]);
    assert_eq!(body(&a), "abXY", "the first side's own-insertion deletion is rolled back with the refused second side");
    assert_eq!(a.session.undo_depth(), depth, "the half-erasure leaves no undo step");
    assert_eq!(a.session.doc.revisions.len(), revisions, "revision metadata is restored too");
    assert!(a.canvas.ime_join_text.is_none(), "no rewrite is promised");
    assert!(a.canvas.ime_interrupt_now, "the platform's stale surrounding snapshot is discarded");
    assert_eq!(a.session.sel.focus.off, 2, "the caret is back where the request found it");
    assert_eq!(a.session.pending, Some(pending), "pending formatting is restored");
    assert_eq!(a.session.goal_x, Some(123.0), "the Up/Down goal column is restored");
}

/// Selects a body range, wraps it in a rich-text content control with `tag`, and sets its
/// placeholder.
fn wrap_in_control(a: &mut WordApp, start: usize, end: usize, tag: &str, placeholder: &str) {
    a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, start), focus: wordcraft_doc::Pos::body(0, end) };
    a.run("developer.richText", json!({"tag": tag})).unwrap();
    a.run("developer.properties", json!({"control": tag, "placeholder": placeholder})).unwrap();
}

/// Finds the body content control with `tag`, panicking if it is absent.
fn control(a: &WordApp, tag: &str) -> wordcraft_doc::ControlRange {
    a.session
        .doc
        .control_ranges(wordcraft_doc::StoryRef::Body)
        .into_iter()
        .find(|r| r.control.tag == tag)
        .unwrap_or_else(|| panic!("the {tag} control remains"))
}

/// Emptying a control restores a placeholder longer than the erased text. The right-hand
/// range and the caret must move by the *signed* length change: the `X` after the control is
/// erased (not a placeholder character), the caret lands at the placeholder's end, and the
/// commit replaces the placeholder in the same undo step as the deletion.
#[test]
fn delete_surrounding_maps_after_text_and_caret_past_a_growing_placeholder() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "Name: AdaX"})).unwrap();
    wrap_in_control(&mut a, 6, 9, "name", "Enter the full name");
    let r = control(&a, "name");
    let placeholder = r.control.placeholder_shown().to_string();
    assert!(placeholder.len() > "Ada".len(), "the fixture grows the paragraph: {placeholder:?}");
    a.session.sel = wordcraft_engine::Selection::caret(r.end.clone());
    let depth = a.session.undo_depth();

    alt_frame(&ctx, &mut a, vec![surround(3, 1)]);
    assert_eq!(body(&a), format!("Name: {placeholder}"), "the following `X` is erased, not a placeholder character");
    let end = control(&a, "name").content_start().off + placeholder.len();
    assert_eq!((a.session.sel.anchor.off, a.session.sel.focus.off), (end, end), "the caret is at the placeholder's end, never inside it");
    assert_eq!(a.session.undo_depth(), depth + 1, "both erased sides are one undo step");

    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("N".into()))]);
    assert_eq!(body(&a), "Name: N", "the commit replaces the restored placeholder");
    assert_eq!(a.session.undo_depth(), depth + 1, "the replacement joins the deletion");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "Name: AdaX", "one Undo restores both erased sides and the replacement");
}

/// A placeholder shorter than the erased text also has to map the suffix and caret by the
/// signed delta; subtracting a removed-byte count alone describes only this direction.
#[test]
fn delete_surrounding_maps_a_shrinking_placeholder_and_after_text() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "Name: AdaX"})).unwrap();
    wrap_in_control(&mut a, 6, 9, "name", "N");
    let r = control(&a, "name");
    a.session.sel = wordcraft_engine::Selection::caret(r.end.clone());
    let depth = a.session.undo_depth();

    alt_frame(&ctx, &mut a, vec![surround(3, 1)]);
    assert_eq!(body(&a), "Name: N", "the shorter placeholder replaces `Ada`, and `X` still goes");
    let end = control(&a, "name").content_start().off + 1;
    assert_eq!((a.session.sel.anchor.off, a.session.sel.focus.off), (end, end));
    assert_eq!(a.session.undo_depth(), depth + 1);

    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("Z".into()))]);
    assert_eq!(body(&a), "Name: Z");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "Name: AdaX", "one Undo restores the original value and the following text");
}

/// The after side alone can empty a control and restore a longer placeholder. Its caret
/// must map to the restored placeholder's end, not to the old pre-deletion offset, and the
/// text beyond the after-side range stays put.
#[test]
fn delete_surrounding_maps_a_placeholder_restored_by_the_after_side() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "aXY"})).unwrap();
    wrap_in_control(&mut a, 1, 2, "name", "Enter the value");
    let r = control(&a, "name");
    let placeholder = r.control.placeholder_shown().to_string();
    assert!(placeholder.len() > 1);
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1));
    let depth = a.session.undo_depth();

    alt_frame(&ctx, &mut a, vec![surround(0, 1)]);
    assert_eq!(body(&a), format!("a{placeholder}Y"), "only the one after-side `X` is erased; `Y` is beyond the range");
    let end = control(&a, "name").content_start().off + placeholder.len();
    assert_eq!((a.session.sel.anchor.off, a.session.sel.focus.off), (end, end));
    assert_eq!(a.session.undo_depth(), depth + 1);

    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))]);
    assert_eq!(body(&a), "aQY", "the commit replaces the placeholder and leaves the following `Y`");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "aXY", "one Undo restores the emptied control, the following text and the commit");
}

/// A refused body request clears the locally painted pre-edit and every replacement join.
/// The earlier successful deletion stays, but the following commit gets its own Undo step.
#[test]
fn a_refused_body_delete_clears_preedit_and_joins() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "ab"})).unwrap();
    wrap_in_control(&mut a, 1, 2, "locked", "b");
    a.run("developer.properties", json!({"control": "locked", "lockEdit": true})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1));
    let depth = a.session.undo_depth();

    alt_frame(
        &ctx,
        &mut a,
        vec![surround(1, 0), egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None }), surround(0, 1)],
    );
    assert_eq!(body(&a), "b", "the editable `a` is erased; the locked control's `b` is not");
    assert!(a.canvas.ime_preedit.is_empty(), "a refused request must not leave its pre-edit painted");
    assert_eq!(a.canvas.ime_preedit_range, None);
    assert!(a.canvas.ime_join_text.is_none(), "no replacement formatting is promised");
    assert!(a.canvas.ime_interrupt_now, "the platform's stale surroundings are interrupted");
    assert_eq!(a.session.undo_depth(), depth + 1, "only the accepted first deletion has a step");

    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
    assert_eq!(body(&a), "Xb");
    assert_eq!(a.session.undo_depth(), depth + 2, "the cleared join gives the commit its own checkpoint");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "b", "Undo takes back only the commit, not the earlier deletion");
}

/// A body deletion promises its join only to the immediate matching replacement. An ordinary
/// mutating command in between consumes the promise and keeps its own Undo checkpoint.
#[test]
fn body_delete_join_does_not_leak_to_an_intervening_command() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "abcd"})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 4));
    let base = a.session.undo_depth();

    alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
    assert_eq!(body(&a), "abc");
    assert_eq!(a.session.undo_depth(), base + 1, "the deletion is its own step");

    a.run("format.bold", json!({})).unwrap();
    assert_eq!(body(&a), "abc", "the intervening command changes no text");
    assert_eq!(a.session.undo_depth(), base + 2, "an intervening ordinary command owns its checkpoint");

    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
    assert_eq!(body(&a), "abcX", "the replacement still applies");
    assert_eq!(a.session.undo_depth(), base + 3, "the replacement no longer joins across the ordinary command");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "abc", "Undo takes back only the commit, not the deletion or the Bold step");
}

/// A cross-block selection has no single surrounding snapshot. Refusing the request must
/// also interrupt the composition instead of leaving the platform's hidden text stale.
#[test]
fn cross_block_delete_surrounding_is_refused_and_interrupts() {
    let (ctx, mut a) = alt_frame_app();
    a.run("document.setText", json!({"text": "one\ntwo"})).unwrap();
    a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(1, 1) };
    alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
    assert_eq!(body(&a), "one\ntwo", "nothing is deleted across blocks");
    assert!(a.canvas.ime_interrupt_now, "the refused request interrupts the composition");
}

/// An IME replacement inside an equation erases the counted characters with the equation's
/// commands; a byte count (which has no mapping into the tree) deletes nothing. A refusal
/// drops only the replacement immediately paired with it: the next `Commit`, or an adjacent
/// `Text` + `Commit`. A later key, pre-edit or direct commit in the same raw batch is fresh
/// input and applies; so does one in a later frame.
#[test]
fn ime_equation_byte_delete_drops_only_its_immediate_replacement() {
    let (ctx, mut a) = alt_frame_app();
    a.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    let linear = |a: &mut WordApp| a.run("equation.get", json!({})).unwrap().get("linear").and_then(|v| v.as_str()).unwrap_or("").to_string();
    assert_eq!(linear(&mut a), "x+y");
    alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
    assert_eq!(linear(&mut a), "x+", "the character before the caret in the equation is erased");
    let base = linear(&mut a);

    // A direct commit in a later frame is fresh input, not the stale replacement.
    a.canvas.ime_interrupt_now = false;
    alt_frame(&ctx, &mut a, vec![surround_bytes(1, 0)]);
    assert_eq!(linear(&mut a), base, "a byte count is not guessed into equation characters");
    assert!(a.canvas.ime_interrupt_now, "the refused byte deletion interrupts the composition");
    a.canvas.ime_interrupt_now = false;
    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("D".into()))]);
    assert_eq!(linear(&mut a), format!("{base}D"), "a later direct commit applies without a pre-edit");

    // The immediately following commit belongs to the refused replacement and is dropped;
    // the next commit in the same raw batch is fresh input and applies.
    let after_pair = linear(&mut a);
    a.canvas.ime_interrupt_now = false;
    alt_frame(
        &ctx,
        &mut a,
        vec![surround_bytes(1, 0), egui::Event::Ime(egui::ImeEvent::Commit("Z".into())), egui::Event::Ime(egui::ImeEvent::Commit("Y".into()))],
    );
    assert_eq!(linear(&mut a), format!("{after_pair}Y"), "only the adjacent stale commit is dropped");
    let after_commit = linear(&mut a);

    // A platform that routes the replacement through `Text` before its `Commit` sends the
    // adjacent pair; both belong to the refused deletion, and a later commit still applies.
    a.canvas.ime_interrupt_now = false;
    alt_frame(
        &ctx,
        &mut a,
        vec![
            surround_bytes(1, 0),
            egui::Event::Text("Q".into()),
            egui::Event::Ime(egui::ImeEvent::Commit("Q2".into())),
            egui::Event::Ime(egui::ImeEvent::Commit("R".into())),
        ],
    );
    assert_eq!(linear(&mut a), format!("{after_commit}R"), "the adjacent Text+Commit pair is stale, the next commit is not");

    // Any intervening event ends the pair: a key then its Text is ordinary fresh input.
    let after_commit = linear(&mut a);
    a.canvas.ime_interrupt_now = false;
    let key = egui::Event::Key { key: Key::F20, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    alt_frame(&ctx, &mut a, vec![surround_bytes(1, 0), key, egui::Event::Text("K".into())]);
    assert_eq!(linear(&mut a), format!("{after_commit}K"), "a key between the deletion and text clears the suppression");

    // A pre-edit likewise ends the stale pair; its commit is fresh.
    let after_commit = linear(&mut a);
    a.canvas.ime_interrupt_now = false;
    alt_frame(
        &ctx,
        &mut a,
        vec![
            surround_bytes(1, 0),
            egui::Event::Ime(egui::ImeEvent::Preedit { text: "p".into(), active_range_chars: None }),
            egui::Event::Ime(egui::ImeEvent::Commit("P".into())),
        ],
    );
    assert_eq!(linear(&mut a), format!("{after_commit}P"), "a fresh composition in the same batch is accepted");
}

/// An over-cap equation deletion is still a new surrounding request: it discards the
/// composition that was painted for the snapshot it refuses, and interrupts it.
#[test]
fn over_cap_equation_delete_clears_the_painted_preedit() {
    let (ctx, mut a) = alt_frame_app();
    a.run("insert.equation", json!({"linear": "abc"})).unwrap();
    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "p".into(), active_range_chars: None })]);
    assert_eq!(a.canvas.ime_preedit, "p");
    a.canvas.ime_interrupt_now = false;
    alt_frame(&ctx, &mut a, vec![surround(wordcraft_engine::MAX_EQUATION_DELETE_UNITS + 1, 0)]);
    assert_eq!(body(&a), "abc", "the over-cap request deletes nothing");
    assert!(a.canvas.ime_preedit.is_empty(), "the refused request's pre-edit is not left painted");
    assert_eq!(a.canvas.ime_preedit_range, None);
    assert!(a.canvas.ime_interrupt_now, "the platform's stale snapshot is interrupted");
}

/// An over-cap equation deletion also clears a previous equation deletion's promised join,
/// so the next commit cannot silently merge into that earlier undo step.
#[test]
fn over_cap_equation_delete_clears_the_previous_replacement_join() {
    let (ctx, mut a) = alt_frame_app();
    a.run("insert.equation", json!({"linear": "abcd"})).unwrap();
    alt_frame(&ctx, &mut a, vec![surround(1, 0), surround(wordcraft_engine::MAX_EQUATION_DELETE_UNITS + 1, 0)]);
    assert_eq!(body(&a), "abc", "the first, valid deletion stands");
    assert!(a.canvas.ime_interrupt_now, "the over-cap request interrupts the composition");
    let depth = a.session.undo_depth();
    a.canvas.ime_interrupt_now = false;
    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
    assert_eq!(body(&a), "abcX");
    assert_eq!(a.session.undo_depth(), depth + 1, "the cleared join gives the commit its own undo step");
}

/// The transactional equation delete and the following commit are one undo step, like the
/// body's `DeleteSurrounding` plus its replacement text.
#[test]
fn ime_equation_delete_and_commit_are_one_undo_step() {
    let (ctx, mut a) = alt_frame_app();
    a.run("insert.equation", json!({"linear": "abcd"})).unwrap();
    alt_frame(&ctx, &mut a, vec![surround(1, 0), egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
    assert_eq!(body(&a), "abcX");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "abcd", "one Undo restores the deleted unit and the commit together");
}

/// An equation deletion promises a join only to its immediate replacement. An unrelated
/// command in between consumes the promise without joining, so it gets its own undo step
/// instead of being merged into the deletion.
#[test]
fn ime_equation_delete_join_does_not_leak_to_an_unrelated_command() {
    let (ctx, mut a) = alt_frame_app();
    a.run("insert.equation", json!({"linear": "abcd"})).unwrap();
    let base = a.session.undo_depth();
    alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
    assert_eq!(body(&a), "abc");
    assert_eq!(a.session.undo_depth(), base + 1, "the deletion is its own undo step");

    a.run("equation.justify", json!({"jc": "right"})).unwrap();
    assert_eq!(a.session.undo_depth(), base + 2, "an unrelated command keeps its own checkpoint");

    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
    assert_eq!(body(&a), "abcX");
    assert_eq!(a.session.undo_depth(), base + 3, "the replacement no longer joins across the unrelated command");

    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(body(&a), "abc", "Undo takes back only the commit, not the deletion and the unrelated command");
}

/// Hostile pre-edit and commit sizes are refused before they reach the canvas or the model.
#[test]
fn oversized_ime_events_are_ignored() {
    let (ctx, mut a) = alt_frame_app();
    let huge = "a".repeat(MAX_IME_PREEDIT_BYTES + 1);
    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: huge, active_range_chars: Some(0..1) })]);
    assert!(a.canvas.ime_preedit.is_empty(), "an oversized pre-edit is not painted");
    assert_eq!(a.canvas.ime_preedit_range, None);
    let huge = "b".repeat(MAX_IME_COMMIT_BYTES + 1);
    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit(huge))]);
    assert_eq!(body(&a), "", "an oversized commit is not inserted");
    // The equation cap is lower than the byte cap: an over-cap commit is refused before the
    // command, with no prefix committed and the composition interrupted.
    a.run("insert.equation", json!({})).unwrap();
    a.canvas.ime_interrupt_now = false;
    let long = "c".repeat(wordcraft_engine::MAX_EQUATION_TYPE_CHARS + 1);
    alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit(long))]);
    assert_eq!(body(&a), "", "no equation prefix is committed");
    assert!(a.canvas.ime_interrupt_now, "the refused commit starts the composition afresh");
}

/// A pre-edit belongs to the editor that had focus: losing it drops the painted composition.
#[test]
fn observe_ime_drops_a_preedit_without_focus() {
    let ctx = egui::Context::default();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.canvas.ime_preedit = "に".into();
    a.canvas.ime_preedit_range = Some(0..1);
    a.canvas.focused = false;
    let r = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0));
    observe_ime(&mut a, &ctx, Some((r, r)), false);
    assert!(a.canvas.ime_preedit.is_empty(), "no composition is kept for an unfocused editor");
    assert_eq!(a.canvas.ime_preedit_range, None);
}

#[test]
fn typing_with_smart_art_selected_types_into_an_item() {
    let ctx = egui::Context::default();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("insert.smartArt", json!({"layout": "matrix", "items": ["Ada", "Bea"]})).unwrap();
    for t in ["x", "y"] {
        ctx.run_ui(egui::RawInput { events: vec![egui::Event::Text(t.into())], ..Default::default() }, |ui| canvas_events(&mut a, ui.ctx()))
            .drop_without_applying_deltas();
    }
    let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&a.session).expect("the graphic stays");
    assert_eq!(g.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["Adaxy", "Bea"]);
    assert!(a.session.view.smart_art_pane, "the Text Pane opens");
    a.run("edit.undo", json!({})).unwrap();
    let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&a.session).expect("still there");
    assert_eq!(g.items[0].text, "Ada", "one undo step for the typing");
}

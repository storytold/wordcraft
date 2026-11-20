use craft::platform::clipboard::{Clipboard, MemoryClipboard};
use craft::ui::primitives::ImeStamp;
use craft::ui::runtime::commands::{ClipItem, ClipboardHandle};

use super::*;

/// The app harness the key tests drive: frames of the real `WordApp` with events pushed by `h.event`.
type H = egui_kittest::Harness<'static, WordApp>;

/// Builds an IME edit outcome test fixture with `selection_changed` false.
fn outcome(classified: bool, content: bool, state: bool, transformed: bool) -> ImeEditOutcome {
    ImeEditOutcome { classified, content_changed: content, state_changed: state, transformed, selection_changed: false }
}

/// The central gate: a classified no-op leaves an earlier receipt alone, while any
/// unprovable mutation forces `Other`.
#[test]
fn ime_receipt_gate_distinguishes_noop_from_unproven_mutation() {
    let same = ImeStamp::from_committed("ab", 2, 2, None);
    assert_eq!(classify_ime_proof(outcome(true, false, false, false), &same, &same), ImeProof::NoMutation);
    let changed = ImeStamp::from_committed("abx", 3, 3, None);
    assert_eq!(classify_ime_proof(outcome(true, true, true, false), &same, &changed), ImeProof::Certified);
    assert_eq!(classify_ime_proof(outcome(true, true, true, true), &same, &changed), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(outcome(false, false, true, false), &same, &changed), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(outcome(true, true, true, false), &same, &same), ImeProof::UnprovenMutation);
}

/// Unavailable receipts need both checked revisions and a strict increase; mixed transitions
/// need the unavailable endpoint's revision.
#[test]
fn ime_receipt_gate_checks_unavailable_revisions() {
    let ten = ImeStamp::unavailable_at_revision(10);
    let eleven = ImeStamp::unavailable_at_revision(11);
    let missing = ImeStamp::unavailable();
    let content = outcome(true, true, true, false);
    assert_eq!(classify_ime_proof(content, &ten, &eleven), ImeProof::Certified);
    assert_eq!(classify_ime_proof(content, &eleven, &ten), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(content, &ten, &ten), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(content, &missing, &eleven), ImeProof::UnprovenMutation);
    let available = ImeStamp::from_committed("abx", 3, 3, None);
    assert_eq!(classify_ime_proof(content, &ten, &available), ImeProof::Certified);
    assert_eq!(classify_ime_proof(content, &missing, &available), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(content, &available, &ten), ImeProof::Certified);
    assert_eq!(classify_ime_proof(content, &available, &missing), ImeProof::UnprovenMutation);
}

/// A revision-only unavailable change is a changed observation, not a no-op.
#[test]
fn ime_receipt_gate_treats_revision_only_changes_as_unproven() {
    let ten = ImeStamp::unavailable_at_revision(10);
    let eleven = ImeStamp::unavailable_at_revision(11);
    assert_eq!(classify_ime_proof(outcome(true, false, false, false), &ten, &eleven), ImeProof::UnprovenMutation);
}

/// An exhausted engine allocator emits `Unavailable` without a revision. It can never prove
/// a content change; a classified no-op still leaves an earlier receipt alone.
#[test]
fn ime_receipt_gate_rejects_exhausted_unavailable_stamps() {
    let exhausted = ImeStamp::unavailable();
    assert_eq!(classify_ime_proof(outcome(true, true, true, false), &exhausted, &exhausted), ImeProof::UnprovenMutation);
    assert_eq!(classify_ime_proof(outcome(true, false, false, false), &exhausted, &exhausted), ImeProof::NoMutation);
}

/// An app whose canvas holds focus, in a harness at 60 Hz. The focus goes through `focus_page`, so
/// the engine's owner rule keeps the IME events of the tests that follow.
fn app_harness(app: WordApp) -> H {
    // Every test isolates through an in-memory clipboard, installed before the first frame.
    let spec = || crate::test_harness::install_spec().clipboard(Clipboard::new(MemoryClipboard::default()));
    let mut h = crate::test_harness::builder_with(spec).with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    for _ in 0..8 {
        crate::test_harness::frame(&mut h);
        if h.state().canvas.focused(&ctx) {
            break;
        }
    }
    assert!(h.state().canvas.focused(&ctx), "the canvas takes the focus it asked for");
    h
}

/// A new empty document.
fn new_app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default())
}

/// A focused harness over an empty document.
fn alt_frame_app() -> H {
    app_harness(new_app())
}

/// One frame with `events` pushed first.
fn alt_frame(h: &mut H, events: Vec<egui::Event>) {
    for e in events {
        h.event(e);
    }
    // Each frame as the app's driver runs it: the runtime is handed the app's commands first.
    crate::test_harness::frame(h);
}

/// One frame whose input carries all of `events`, as a platform delivers one raw batch. `alt_frame` queues them
/// through kittest's `event`, which runs a frame for each queued event, so a batch it is given arrives spread over as
/// many frames.
fn batch_frame(h: &mut H, events: Vec<egui::Event>) {
    h.input_mut().events.extend(events);
    crate::test_harness::frame(h);
}

/// One frame with `modifiers` held, then `events`.
fn mods_frame(h: &mut H, modifiers: Modifiers, events: Vec<egui::Event>) {
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    alt_frame(h, events);
}

/// Runs a command on the harness's app.
fn run(h: &mut H, command: &str, params: serde_json::Value) -> serde_json::Value {
    h.state_mut().run(command, params).unwrap()
}

/// Puts the caret in the first body paragraph at byte `off`.
fn caret(h: &mut H, off: usize) {
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, off));
}

/// Puts `items` on the window's clipboard.
fn offer(h: &H, items: Vec<ClipItem>) {
    let _ = ClipboardHandle::of(&h.ctx).offer(&items).expect("the clipboard takes the offer");
}

/// The clipboard flavours of a bold word: HTML and text.
fn bold_flavours(word: &str) -> Vec<ClipItem> {
    vec![ClipItem::Html(format!("<p><b>{word}</b></p>").into()), ClipItem::Text(word.into())]
}

/// Whether the state of the app's format at the caret says bold.
fn is_bold(h: &mut H) -> bool {
    run(h, "format.state", json!({})).get("bold").and_then(serde_json::Value::as_bool) == Some(true)
}

/// The holder of a chord on the board, as a command.
fn holder_of(h: &H, text: &str) -> Option<craft::keymap::CommandRef> {
    craft::ui::runtime::commands::holder(&h.ctx, &craft::keymap::Chord::parse(text).unwrap())
}

/// #1678 and shell-w6's verdict: an assignment becomes an override that takes the chord from its
/// default holder; Ctrl+F1 gives way to a binding, Alt+Q does not (off macOS); interface chords
/// are per platform. The window's platform is set, so both run on every host.
#[test]
fn user_assignments_become_overrides() {
    use craft::host::target::Os;
    use craft::keymap::CommandRef;
    let command = |id: &str| Some(CommandRef::new(id));
    let tell_me = Some(CommandRef::with_params("ui.dialog", json!({"name": "commands"})));
    for os in [Os::Mac, Os::Windows] {
        let mac = os == Os::Mac;
        // (assignment: chord and command, chord to ask, what holds it afterwards)
        let rows = [
            (None, "Mod+F1", if mac { None } else { command("ui.collapseRibbon") }),
            // The engine binds Mod+Alt+R to the Registered Sign off macOS (decided when it is built, not by the
            // window's platform), so a build for macOS is the only one where the Mac window's chord is the ribbon's.
            (
                None,
                "Mod+Alt+R",
                if !cfg!(target_os = "macos") {
                    command("text.registered")
                } else if mac {
                    command("ui.collapseRibbon")
                } else {
                    None
                },
            ),
            // Likewise the engine binds Alt+Q to Search off macOS when it is built, so a Mac window on such a build
            // still finds it held; only a build for macOS leaves the chord free.
            (
                None,
                "Alt+Q",
                if !mac {
                    tell_me.clone()
                } else if !cfg!(target_os = "macos") {
                    command("tools.search")
                } else {
                    None
                },
            ),
            (Some(("Mod+F1", "view.focus")), "Mod+F1", command("view.focus")),
            (Some(("Alt+Q", "view.focus")), "Alt+Q", if mac { command("view.focus") } else { tell_me.clone() }),
        ];
        for (assign, ask, want) in rows {
            let mut h = alt_frame_app();
            craft::ui::testing::windows::set_os(&h.ctx, os);
            crate::test_harness::frame(&mut h);
            if let Some((chord, id)) = assign {
                h.state_mut().session.keymap_mut().insert(CommandRef::new(id).key(), vec![chord.to_string()]);
                crate::test_harness::frame(&mut h);
            }
            assert_eq!(holder_of(&h, ask), want, "{os:?} {assign:?} {ask}");
        }
        // A bound chord leaves its default holder for the command it is bound to.
        let mut h = alt_frame_app();
        craft::ui::testing::windows::set_os(&h.ctx, os);
        crate::test_harness::frame(&mut h);
        let default = holder_of(&h, "Mod+Shift+Backspace");
        assert!(default.is_some() && default != command("select.all"), "{os:?}: the chord starts with its own command");
        // The way the Customize Keyboard dialog assigns: the board's keymap takes the chord from its holder
        // (`Policy::Steal`), and its overrides become the session's. A raw override entry only ties with the
        // default holder, which the catalog order settles.
        let mut keymap = craft::ui::runtime::commands::keymap(&h.ctx).map(|board| (*board).clone()).expect("the board has a keymap");
        let chord = craft::keymap::Binding::parse("Mod+Shift+Backspace").unwrap();
        keymap.assign(&CommandRef::new("select.all"), chord, craft::keymap::Policy::Steal).unwrap();
        *h.state_mut().session.keymap_mut() = keymap.overrides().clone();
        crate::test_harness::frame(&mut h);
        assert_eq!(holder_of(&h, "Mod+Shift+Backspace"), command("select.all"), "{os:?}");
    }
}

/// #5616: Mod+Shift+V pastes once, as text only (the word is not bold) though the clipboard holds
/// bold HTML, whichever of Shift and V is released first.
#[test]
fn mod_shift_v_released_shift_first_pastes_once() {
    let chord = Modifiers { ctrl: true, command: true, shift: true, ..Default::default() };
    let ctrl = Modifiers { shift: false, ..chord };
    let mut h = alt_frame_app();
    run(&mut h, "text.insert", json!({"text": "source target"}));
    run(&mut h, "select.text", json!({"text": "target"}));
    offer(&h, bold_flavours("source"));
    mods_frame(&mut h, chord, vec![key_event(Key::V, true, chord)]);
    mods_frame(&mut h, ctrl, vec![key_event(Key::ShiftLeft, false, ctrl)]);
    mods_frame(&mut h, ctrl, vec![key_event(Key::V, false, ctrl)]);
    assert_eq!(body(h.state()), "source source");
    assert!(!is_bold(&mut h), "Keep Text Only");
}

/// #5616: Mod+Shift+V, then the next Mod+V: the picture on the clipboard still pastes.
#[test]
fn a_global_mod_shift_v_leaves_the_next_mod_v_picture_paste_alone() {
    let mut h = picture_app();
    let chord = Modifiers { ctrl: true, command: true, shift: true, ..Default::default() };
    mods_frame(&mut h, chord, vec![key_event(Key::V, true, chord)]);
    let ctrl = Modifiers { shift: false, ..chord };
    mods_frame(&mut h, ctrl, vec![key_event(Key::V, false, ctrl)]);
    let before = pictures(h.state_mut());
    mods_frame(&mut h, ctrl, vec![key_event(Key::V, true, ctrl)]);
    mods_frame(&mut h, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
    assert_eq!(pictures(h.state_mut()), before + 1);
}

/// A focused empty document whose clipboard holds a 4 × 4 picture.
fn picture_app() -> H {
    let h = alt_frame_app();
    offer(&h, vec![ClipItem::Image(std::sync::Arc::new(egui::ColorImage::from_rgba_unmultiplied([4, 4], &[200u8; 4 * 4 * 4])))]);
    h
}

/// How many pictures the document holds (the Selection Pane's list).
fn pictures(a: &mut WordApp) -> usize {
    a.session.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(|x| x.len()).unwrap_or(0)
}

/// A source word in bold with Copy Formatting picked up, and "target" selected.
fn copied_bold_app() -> WordApp {
    let mut a = new_app();
    a.run("text.insert", json!({"text": "source target"})).unwrap();
    a.run("select.text", json!({"text": "source"})).unwrap();
    a.run("format.bold", json!({})).unwrap();
    a.run("edit.copyFormat", json!({})).unwrap();
    a.run("select.text", json!({"text": "target"})).unwrap();
    a
}

/// Paste Formatting on a chord the user bound pastes the copied formatting once.
#[test]
fn paste_formatting_on_another_key_pastes_the_copied_formatting() {
    let mut app = copied_bold_app();
    app.session.keymap_mut().insert(craft::keymap::CommandRef::new("edit.pasteFormat").key(), vec!["Mod+Alt+P".to_string()]);
    let mut h = app_harness(app);
    let m = Modifiers::COMMAND | Modifiers::ALT;
    mods_frame(&mut h, m, vec![key_event(Key::P, true, m)]);
    mods_frame(&mut h, Modifiers::NONE, vec![key_event(Key::P, false, Modifiers::NONE)]);
    assert_eq!(body(h.state()), "source target", "the text stays");
    assert!(is_bold(&mut h), "the formatting was pasted onto the selection");
}

/// A press or release of `key` with `modifiers`.
fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
}

/// Mod+V keeps the HTML's bold; with a picture on the clipboard it pastes the picture once,
/// whether V or Ctrl is released first, and a plain V is no paste.
#[test]
fn mod_v_released_mod_first_pastes_the_clipboard_picture() {
    let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };

    let mut h = alt_frame_app();
    run(&mut h, "text.insert", json!({"text": "x"}));
    run(&mut h, "select.all", json!({}));
    offer(&h, bold_flavours("external"));
    mods_frame(&mut h, ctrl, vec![key_event(Key::V, true, ctrl)]);
    mods_frame(&mut h, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
    assert_eq!(body(h.state()), "external");
    assert!(is_bold(&mut h), "Mod+V keeps the HTML's formatting");

    let mut h = picture_app();
    mods_frame(&mut h, ctrl, vec![key_event(Key::ControlLeft, true, ctrl), key_event(Key::V, true, ctrl)]);
    mods_frame(&mut h, Modifiers::NONE, vec![key_event(Key::ControlLeft, false, Modifiers::NONE)]);
    mods_frame(&mut h, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
    assert_eq!(pictures(h.state_mut()), 1);
    // A plain V afterwards is not a paste.
    mods_frame(&mut h, Modifiers::NONE, vec![key_event(Key::V, true, Modifiers::NONE)]);
    mods_frame(&mut h, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
    assert_eq!(pictures(h.state_mut()), 1);
}

/// Alt held, keypad 0 2 3 3, Alt released: é (й on a Russian system) is typed, the digits
/// aren't, and keytips stay off (#961). Linux sends each digit's text too; Windows may send the
/// composed character.
#[test]
fn alt_numpad_code_types_its_character_without_keytips() {
    if cfg!(target_os = "macos") {
        return;
    }
    let mut h = alt_frame_app();
    // Windows reads codes in the system locale's code pages (0233 is é in 1252 and 1250, й in
    // 1251); elsewhere the ANSI page is 1252 and the OEM page the locale's own.
    let (ansi, oem_page) = if cfg!(target_os = "windows") {
        wordcraft_engine::cmd::text::system_code_pages()
    } else {
        (1252, wordcraft_engine::cmd::text::system_oem_code_page())
    };
    let typed = craft::codepage::alt_code_char("0233", ansi, oem_page).unwrap_or('?');
    let alt = Modifiers { alt: true, ..Default::default() };
    alt_frame(&mut h, vec![egui::Event::ModifiersChanged(alt), key_event(Key::AltLeft, true, alt)]);
    for (k, t) in [(Key::Num0, "0"), (Key::Num2, "2"), (Key::Num3, "3"), (Key::Num3, "3")] {
        alt_frame(&mut h, vec![key_event(k, true, alt), egui::Event::Text(t.into()), key_event(k, false, alt)]);
    }
    assert_eq!(body(h.state()), "", "nothing typed while Alt is held");
    alt_frame(&mut h, vec![key_event(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(body(h.state()), typed.to_string());
    assert!(!h.state().keytips_active, "an Alt code doesn't open keytips");
    // The platform's own composed character isn't typed twice; the next typing is.
    alt_frame(&mut h, vec![egui::Event::Text(typed.to_string())]);
    alt_frame(&mut h, vec![key_event(Key::E, true, Modifiers::NONE), egui::Event::Text("é".into())]);
    assert_eq!(body(h.state()), format!("{typed}é"));
    // Without the leading zero: the system locale's OEM code page.
    alt_frame(&mut h, vec![egui::Event::ModifiersChanged(alt), key_event(Key::AltLeft, true, alt)]);
    for k in [Key::Num2, Key::Num3, Key::Num3] {
        alt_frame(&mut h, vec![key_event(k, true, alt), key_event(k, false, alt)]);
    }
    alt_frame(&mut h, vec![key_event(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
    let oem = craft::codepage::alt_code_char("233", ansi, oem_page);
    assert_eq!(body(h.state()), format!("{typed}é{}", oem.unwrap_or('?')));
    // A bare Alt tap still opens keytips.
    alt_frame(&mut h, vec![egui::Event::ModifiersChanged(alt), key_event(Key::AltLeft, true, alt)]);
    alt_frame(&mut h, vec![key_event(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
    assert!(h.state().keytips_active);
}

/// #2388: AltGr (Right Alt on a Polish layout under Wayland) arrives as the Right Alt key
/// followed by a modifiers event without Alt (it's Level 3 Shift). That must not open keytips
/// while AltGr is held, and a bare AltGr tap doesn't either; a Right Alt that is Alt (US layout)
/// still does.
#[test]
fn altgr_types_its_characters_without_keytips() {
    if cfg!(target_os = "macos") {
        return;
    }
    let mut h = alt_frame_app();
    let none = Modifiers::NONE;
    let altgr_down = || vec![key_event(Key::AltRight, true, none), egui::Event::ModifiersChanged(none)];
    let altgr_up = || vec![key_event(Key::AltRight, false, none), egui::Event::ModifiersChanged(none)];
    alt_frame(&mut h, altgr_down());
    assert!(!h.state().keytips_active, "AltGr down doesn't open keytips");
    for (k, t) in [(Key::S, "ś"), (Key::L, "ł"), (Key::Z, "ż")] {
        alt_frame(&mut h, vec![key_event(k, true, none), egui::Event::Text(t.into())]);
        alt_frame(&mut h, vec![key_event(k, false, none)]);
        assert!(!h.state().keytips_active, "{t}");
    }
    alt_frame(&mut h, altgr_up());
    assert_eq!(body(h.state()), "śłż");
    assert!(!h.state().keytips_active);
    // All in one frame.
    alt_frame(
        &mut h,
        [altgr_down(), vec![key_event(Key::A, true, none), egui::Event::Text("ą".into()), key_event(Key::A, false, none)], altgr_up()].concat(),
    );
    assert_eq!(body(h.state()), "śłżą");
    assert!(!h.state().keytips_active);
    // A bare AltGr tap, in two frames or one.
    alt_frame(&mut h, altgr_down());
    alt_frame(&mut h, altgr_up());
    assert!(!h.state().keytips_active, "an AltGr tap isn't an Alt tap");
    alt_frame(&mut h, [altgr_down(), altgr_up()].concat());
    assert!(!h.state().keytips_active);
    // The framework reads every Right Alt key as AltGr, whichever of the key and the modifiers event comes first
    // (its key stage's `RightAlt`), so a Right Alt that reports Alt opens no keytips either; the Left Alt does.
    let alt = Modifiers { alt: true, ..Default::default() };
    alt_frame(&mut h, vec![key_event(Key::AltRight, true, none), egui::Event::ModifiersChanged(alt)]);
    alt_frame(&mut h, vec![key_event(Key::AltRight, false, alt), egui::Event::ModifiersChanged(none)]);
    assert!(!h.state().keytips_active, "a Right Alt tap is AltGr's");
    alt_frame(&mut h, vec![egui::Event::ModifiersChanged(alt), key_event(Key::AltLeft, true, alt)]);
    alt_frame(&mut h, vec![key_event(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(none)]);
    assert!(h.state().keytips_active, "a Left Alt tap still opens keytips");
}

/// Option+Shift+- types an em dash outside Outline view, where Word's Alt+Shift+- (Collapse)
/// can't run, so its shortcut doesn't swallow the character (#1933).
#[test]
fn disabled_alt_shortcut_leaves_the_typed_character() {
    let mut h = alt_frame_app();
    let alt_shift = Modifiers { alt: true, shift: true, ..Default::default() };
    let key = |pressed| key_event(Key::Minus, pressed, alt_shift);
    batch_frame(&mut h, vec![key(true), egui::Event::Text("\u{2014}".into()), key(false)]);
    assert_eq!(body(h.state()), "\u{2014}");
}

fn body(a: &WordApp) -> String {
    a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end_matches(['\r', '\n']).to_string()
}

fn surround(before_chars: usize, after_chars: usize) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, after_chars })
}

/// An IME pre-edit event with no active range.
fn preedit(text: &str) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::Preedit { text: text.into(), active_range_chars: None })
}

/// An IME commit event.
fn commit(text: &str) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::Commit(text.into()))
}

/// Whether the last frame asked the input method to start afresh.
fn interrupted(h: &H) -> bool {
    h.output().platform_output.ime.as_ref().is_some_and(|ime| ime.should_interrupt_composition)
}

/// Whether the last frame told the platform its surrounding snapshot is stale. The framework interrupts a
/// composition it saw (a pre-edit); with none to interrupt, the frame's output names the application as the cause
/// of the change (`ImeChangeCause::Other`) beside the surrounding text, which is how the platform resyncs.
fn resynced(h: &H) -> bool {
    interrupted(h) || h.output().platform_output.ime.as_ref().is_some_and(|ime| matches!(ime.change_cause, egui::output::ImeChangeCause::Other))
}

/// Answers the reset an interrupted composition asked the platform for, as a platform does. Until it does the
/// framework drops the old composition's late events, so a test that goes on typing answers first.
fn complete_reset(h: &mut H) {
    let request = h.output().platform_output.ime.as_ref().and_then(|ime| ime.reset_request);
    if let Some(request_id) = request {
        alt_frame(h, vec![egui::Event::ImeResetCompleted { request_id }]);
    }
}

/// A real body replacement invalidates the old preedit even if its text is unchanged.
/// Its queued replacement must not edit the new body or add an Undo checkpoint.
#[test]
fn ime_interrupted_body_preedit_drops_queued_replacement_before_mutation() {
    for replacement in [
        vec![commit("stale")],
        vec![surround(1, 1), commit("stale")],
        vec![surround_bytes(1, 1), commit("stale")],
        // The platform's `Text` + `Commit` pair reaches the page as the commit (`ImeCommit`, then its `Text`): the
        // framework hands the input method's edits over before any keyboard text, so a bare `Text` cannot
        // come ahead of the commit it belongs to and the older ordering is not expressible.
        vec![surround(1, 1), commit("stale"), commit("")],
    ] {
        let mut h = alt_frame_app();
        run(&mut h, "document.setText", json!({"text": "ABCDE"}));
        caret(&mut h, 2);
        alt_frame(&mut h, vec![preedit("に")]);
        assert_eq!(h.state().canvas.ime_preedit, "に");
        run(&mut h, "document.setText", json!({"text": "ABCDE"}));
        caret(&mut h, 2);
        let depth = h.state().session.undo_depth();
        alt_frame(&mut h, replacement);
        assert_eq!(body(h.state()), "ABCDE", "queued events cannot replace text in the new body");
        assert_eq!(h.state().session.sel.focus.off, 2, "not even the new caret moves");
        assert_eq!(h.state().session.undo_depth(), depth, "rejection happens before any transaction");
        assert!(h.state().canvas.ime_preedit.is_empty());
    }
}

/// Exiting the equation through a real key in the same raw batch invalidates its preedit
/// before the following body deletion/commit, as does an external target change.
#[test]
fn ime_interrupted_equation_preedit_drops_queued_body_replacement() {
    // The framework swallows Escape (and Enter, Space, Backspace, Delete) while a composition is on, and hands the
    // input method's edits over before any key, so a key that exits the equation cannot come ahead of the queued
    // replacement in one batch: only the external exit remains to test.
    for in_batch in [false, true] {
        if in_batch {
            continue;
        }
        let mut h = alt_frame_app();
        run(&mut h, "insert.equation", json!({"linear": "abc"}));
        alt_frame(&mut h, vec![preedit("に")]);
        assert_eq!(h.state().canvas.ime_preedit, "に");
        let mut events = Vec::new();
        if in_batch {
            events.push(egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        } else {
            run(&mut h, "equation.exit", json!({}));
        }
        let before = body(h.state());
        let depth = h.state().session.undo_depth();
        events.extend([surround(1, 0), commit("stale")]);
        alt_frame(&mut h, events);
        assert!(h.state().session.math.is_none(), "the equation exit itself still takes effect");
        assert_eq!(body(h.state()), before, "the equation's queued replacement cannot edit the body");
        assert_eq!(h.state().session.undo_depth(), depth, "neither rejected event creates an Undo step");
    }
}

/// Untagged commits are legitimate without an interrupted composition, after a fresh
/// preedit, after the one stale replacement, and after an empty intervening batch.
#[test]
fn ime_interrupted_preedit_does_not_suppress_fresh_input() {
    for fresh_case in 0..4 {
        let mut h = alt_frame_app();
        run(&mut h, "document.setText", json!({"text": "ABCDE"}));
        if fresh_case != 0 {
            alt_frame(&mut h, vec![preedit("old")]);
        }
        run(&mut h, "document.setText", json!({"text": "ABCDE"}));
        caret(&mut h, 2);
        let depth = h.state().session.undo_depth();
        let mut events = Vec::new();
        match fresh_case {
            1 => events.push(preedit("fresh")),
            2 => events.push(commit("stale")),
            3 => {
                alt_frame(&mut h, vec![]);
                // The empty frame interrupts the old composition; the platform answers before fresh input.
                complete_reset(&mut h);
            }
            _ => {}
        }
        events.push(commit("X"));
        alt_frame(&mut h, events);
        assert_eq!(body(h.state()), "ABXCDE", "fresh case {fresh_case} must remain eligible");
        assert_eq!(h.state().session.undo_depth(), depth + 1);
        run(&mut h, "edit.undo", json!({}));
        assert_eq!(body(h.state()), "ABCDE");
    }
}

/// The web's text agent keeps what it typed and, when its value changes other than by an
/// addition at the end, sends `DeleteSurrounding` for the changed tail and then the new text
/// (#1480). The document follows the agent's value instead of repeating it.
#[test]
fn delete_surrounding_from_the_web_text_agent_does_not_repeat_text() {
    let mut h = alt_frame_app();
    alt_frame(&mut h, vec![egui::Event::Text("a".into())]);
    alt_frame(&mut h, vec![egui::Event::Text("b".into())]);
    alt_frame(&mut h, vec![egui::Event::Text("м".into())]);
    alt_frame(&mut h, vec![surround(3, 0), egui::Event::Text("иabм".into())]);
    alt_frame(&mut h, vec![surround(4, 0), egui::Event::Text("риabм".into())]);
    assert_eq!(body(h.state()), "риabм");
    // A correction (`teh` -> `the`): erase three characters, type the replacement.
    alt_frame(&mut h, vec![egui::Event::Text(" teh".into())]);
    alt_frame(&mut h, vec![surround(3, 0), egui::Event::Text("the".into())]);
    assert_eq!(body(h.state()), "риabм the");
    // A plain deletion (Android Backspace): no text follows, or an empty one.
    alt_frame(&mut h, vec![surround(1, 0), egui::Event::Text(String::new())]);
    assert_eq!(body(h.state()), "риabм th");
    alt_frame(&mut h, vec![surround(2, 0)]);
    assert_eq!(body(h.state()), "риabм ");
}

/// Builds an `ImeEvent::DeleteSurroundingBytes` event with the given before and after byte counts.
fn surround_bytes(before_bytes: usize, after_bytes: usize) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes })
}

/// macOS and Wayland IME replacement counts UTF-8 bytes and must delete exactly that range.
/// A byte count into the middle of a character is refused rather than slicing the string.
#[test]
fn delete_surrounding_bytes_replaces_exact_utf8_and_joins_one_undo_step() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "aé👍b"}));
    // The caret is before `b`, after the four-byte emoji.
    caret(&mut h, 7);
    alt_frame(&mut h, vec![surround_bytes(4, 0), egui::Event::Text("x".into())]);
    assert_eq!(body(h.state()), "aéxb");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "aé👍b", "delete plus commit is one undo step");

    // A range that starts inside the emoji is not a UTF-8 boundary: nothing changes.
    caret(&mut h, 7);
    alt_frame(&mut h, vec![surround_bytes(3, 0)]);
    assert_eq!(body(h.state()), "aé👍b");
    // Beyond the paragraph is refused too, and never joins the next paragraph.
    alt_frame(&mut h, vec![surround_bytes(0, 99)]);
    assert_eq!(body(h.state()), "aé👍b");
    let depth = h.state().session.undo_depth();
    for (before, after) in [(99, 0), (usize::MAX, 0), (0, usize::MAX)] {
        alt_frame(&mut h, vec![surround_bytes(before, after)]);
        assert_eq!(body(h.state()), "aé👍b", "out-of-range counts must not erase a prefix");
        assert_eq!(h.state().session.undo_depth(), depth);
    }
}

/// A combining accent is its own UTF-8 range; replacing it does not erase the base letter.
/// `document.setText` leaves the caret at the start, so the test moves it past `e` + accent.
#[test]
fn delete_surrounding_bytes_replaces_a_combining_mark_and_accepts_empty_commit() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "e\u{301}"}));
    caret(&mut h, "e\u{301}".len());
    alt_frame(&mut h, vec![surround_bytes(2, 0), egui::Event::Text(String::new())]);
    assert_eq!(body(h.state()), "e");
    // A deletion-only batch and a 0/0 batch stay harmless.
    alt_frame(&mut h, vec![surround_bytes(1, 0)]);
    assert_eq!(body(h.state()), "");
    alt_frame(&mut h, vec![surround_bytes(0, 0)]);
    assert_eq!(body(h.state()), "");
}

/// The real macOS/Wayland path is `ImeEvent::Commit`, not `Event::Text`: the byte deletion
/// and the committed replacement are one edit, and Undo restores the original text.
#[test]
fn ime_commit_replaces_delete_surrounding_bytes_in_one_undo_step() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "e\u{301}"}));
    caret(&mut h, "e\u{301}".len());
    alt_frame(&mut h, vec![surround_bytes("e\u{301}".len(), 0), commit("é")]);
    assert_eq!(body(h.state()), "é", "the combining sequence is replaced by the IME's committed text");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "e\u{301}", "delete plus Commit is one undo step");
}

/// A platform may deliver the `Commit` that replaces a `DeleteSurrounding` in a later raw
/// batch than the deletion. The undo join belongs to the deletion transaction, not to the
/// frame: the commit still shares its checkpoint, so one Undo restores the original input.
#[test]
fn ime_commit_in_a_later_frame_joins_the_delete_surrounding_undo_step() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "teh"}));
    caret(&mut h, 3);
    let depth = h.state().session.undo_depth();

    alt_frame(&mut h, vec![surround_bytes(3, 0)]);
    assert_eq!(body(h.state()), "");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "the deletion is one step");

    alt_frame(&mut h, vec![commit("the")]);
    assert_eq!(body(h.state()), "the", "the later commit replaces the erased text");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "the later commit joins the deletion's checkpoint");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "teh", "one Undo restores the original input");
}

/// An accepted IME commit can also be empty: it finishes composition without adding text.
#[test]
fn ime_empty_commit_finishes_without_inserting() {
    let mut h = alt_frame_app();
    alt_frame(&mut h, vec![commit("")]);
    assert_eq!(body(h.state()), "");
}

/// `after_chars` erases after the caret; both counts are characters, not bytes.
#[test]
fn delete_surrounding_erases_characters_on_both_sides_of_the_caret() {
    let mut h = alt_frame_app();
    run(&mut h, "text.insert", json!({"text": "мир дом"}));
    for _ in 0..3 {
        run(&mut h, "caret.left", json!({}));
    }
    // The caret is after `мир `: two before (`р `) and one after (`д`).
    alt_frame(&mut h, vec![surround(2, 1)]);
    assert_eq!(body(h.state()), "миом");
    alt_frame(&mut h, vec![egui::Event::Text("X".into())]);
    assert_eq!(body(h.state()), "миXом", "the caret stays where the deletion was");
}

/// The counts come from outside: larger than the paragraph they stop at its ends, never
/// joining paragraphs or panicking.
#[test]
fn delete_surrounding_stays_inside_the_paragraph() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "one\ntwo\nthree"}));
    run(&mut h, "caret.docStart", json!({}));
    // After the `t` of `two`.
    for _ in 0..5 {
        run(&mut h, "caret.right", json!({}));
    }
    alt_frame(&mut h, vec![surround(usize::MAX, usize::MAX)]);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one\n\nthree");
    alt_frame(&mut h, vec![surround(5, 5), egui::Event::Text("2".into())]);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one\n2\nthree");
}

/// The agent's rewrite belongs to the typing it corrects: one Undo takes back the typing
/// and its correction together, not a half-erased word.
#[test]
fn delete_surrounding_joins_the_typing_undo_step() {
    let mut h = alt_frame_app();
    alt_frame(&mut h, vec![egui::Event::Text("teh".into())]);
    alt_frame(&mut h, vec![surround(3, 0), egui::Event::Text("the".into())]);
    assert_eq!(body(h.state()), "the");
    // Typing goes on in the same group, not one step per keystroke.
    alt_frame(&mut h, vec![egui::Event::Text(" more".into())]);
    alt_frame(&mut h, vec![surround(4, 0), egui::Event::Text("mores".into())]);
    alt_frame(&mut h, vec![egui::Event::Text("t".into())]);
    assert_eq!(body(h.state()), "the morest");
    assert_eq!(h.state().session.undo_label(), Some("Typing"));
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "");
}

/// The rewritten text keeps the formatting of the text it replaces: a bold `helo` corrected
/// to `hello` stays bold, as typing over a word does.
#[test]
fn delete_surrounding_keeps_the_replaced_texts_formatting() {
    let mut h = alt_frame_app();
    run(&mut h, "text.insert", json!({"text": "plain "}));
    run(&mut h, "format.bold", json!({}));
    alt_frame(&mut h, vec![egui::Event::Text("helo".into())]);
    alt_frame(&mut h, vec![surround(4, 0), egui::Event::Text("hello".into())]);
    assert_eq!(body(h.state()), "plain hello");
    let f = h.state().session.sel.focus.clone();
    let para = h.state().session.doc.para_at(&f).expect("a paragraph");
    let bold: Vec<bool> = para.text.char_indices().map(|(i, _)| para.props_of_char(i).bold == Some(true)).collect();
    assert_eq!(bold, [false, false, false, false, false, false, true, true, true, true, true], "{bold:?}");
}

/// Overtype replaces characters after the caret when typing, but the text an input method
/// sends to replace what it erased only replaces that.
#[test]
fn delete_surrounding_text_does_not_overtype() {
    let mut h = alt_frame_app();
    run(&mut h, "text.insert", json!({"text": "texXY"}));
    for _ in 0..2 {
        run(&mut h, "caret.left", json!({}));
    }
    run(&mut h, "file.options", json!({"overtype": true}));
    alt_frame(&mut h, vec![surround(3, 0), egui::Event::Text("Texas".into())]);
    assert_eq!(body(h.state()), "TexasXY");
}

/// A key pressed between the erasure and the text ends the input method's rewrite: the text
/// is ordinary typing, its own undo step.
#[test]
fn delete_surrounding_rewrite_ends_at_a_key_press() {
    let mut h = alt_frame_app();
    alt_frame(&mut h, vec![egui::Event::Text("ab".into())]);
    // The framework hands the page the input method's edits first, then the keys and text in their order; Delete is
    // a key the page reads (F20 is no editor input), so it stands between the erasure and the text.
    let key = key_event(Key::Delete, true, Modifiers::NONE);
    alt_frame(&mut h, vec![surround(1, 0), key, egui::Event::Text("X".into())]);
    assert_eq!(body(h.state()), "aX");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "a");
}

/// An automatic change while typing (AutoCorrect `(c)` -> `©`) changes the text behind the
/// web's text agent, so its later counts would be wrong: the page asks it to start afresh.
#[test]
fn automatic_change_asks_the_input_method_to_start_afresh() {
    let mut h = alt_frame_app();
    alt_frame(&mut h, vec![egui::Event::Text("Word".into())]);
    alt_frame(&mut h, vec![egui::Event::Text(" ".into())]);
    assert!(!h.state().canvas.ime_reset, "plain typing keeps the agent's text");
    alt_frame(&mut h, vec![egui::Event::Text("(c)".into())]);
    alt_frame(&mut h, vec![egui::Event::Text(" ".into())]);
    assert_eq!(body(h.state()), "Word \u{a9} ");
    assert!(h.state().canvas.ime_reset, "AutoCorrect changed the text");
}

/// Both sides of one `DeleteSurrounding` are one request: when the second `text.backspace`
/// is refused (here by a content control locked against editing), the first half is rolled
/// back and no rewrite or receipt is promised.
#[test]
fn a_refused_second_delete_surrounding_rolls_the_request_back() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "ab"}));
    // Wrap `b` in a rich-text control that refuses edits, leaving `a` editable.
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 2) };
    run(&mut h, "developer.richText", json!({"tag": "locked"}));
    run(&mut h, "developer.properties", json!({"control": "locked", "lockEdit": true}));
    caret(&mut h, 1);
    let depth = h.state().session.undo_depth();
    let pending = wordcraft_doc::props::CharProps { bold: Some(true), ..Default::default() };
    h.state_mut().session.pending = Some(pending.clone());
    h.state_mut().session.goal_x = Some(123.0);
    alt_frame(&mut h, vec![surround(1, 1)]);
    assert_eq!(body(h.state()), "ab", "the applied left side is rolled back with the refused right one");
    assert_eq!(h.state().session.undo_depth(), depth, "no half-erasure is left in Undo");
    assert!(h.state().canvas.ime_join_text.is_none(), "no text rewrite is promised");
    assert!(resynced(&h), "the platform's stale surrounding snapshot is discarded");
    assert_eq!(h.state().session.sel.focus.off, 1, "the caret is back where the request found it");
    assert_eq!(h.state().session.pending, Some(pending), "pending formatting is restored");
    assert_eq!(h.state().session.goal_x, Some(123.0), "the Up/Down goal column is restored");
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
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "zaXY"}));
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 0), focus: wordcraft_doc::Pos::body(0, 2) };
    run(&mut h, "developer.richText", json!({"tag": "fill"}));
    run(&mut h, "select.text", json!({"text": "X"}));
    run(&mut h, "developer.richText", json!({"tag": "locked"}));
    run(&mut h, "developer.properties", json!({"control": "locked", "lockEdit": true}));
    run(&mut h, "review.restrict", json!({"mode": "forms"}));
    let locked = control(h.state(), "locked");
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(locked.content_start());
    h.state_mut().session.pending = None;
    h.state_mut().canvas.ime_reset = false;
    let _ = h.state_mut().session.take_pending_ime_application_change();
    let depth = h.state().session.undo_depth();
    alt_frame(&mut h, vec![surround(1, 0), commit("\"")]);
    assert_eq!(body(h.state()), "zXY", "the locked replacement is refused, the earlier deletion stands");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "only the successful deletion added an undo step");
    let locked = control(h.state(), "locked");
    assert_eq!(h.state().session.sel.focus, locked.content_start(), "the caret stays where the deletion left it");
    assert_eq!(h.state().session.sel.anchor, locked.content_start());
    assert_eq!(h.state().session.pending, None, "the refused replacement leaves no pending formatting from the erased text");
    assert!(h.state().canvas.ime_join_text.is_none(), "the refused replacement promises no rewrite");
    assert!(resynced(&h), "the platform's stale surrounding snapshot is interrupted");
    assert!(!h.state().canvas.ime_reset, "a refused commit is not an AutoFormat change");
    assert!(!h.state_mut().session.take_pending_ime_application_change(), "no application-origin receipt is fabricated");
}

/// Track Changes can remove the author's own insertion from the stored text. The right-hand
/// range and the final caret then move left by what was actually removed: the second side
/// must erase the character the platform named, not the one that took its stored offset.
#[test]
fn tracked_delete_surrounding_shifts_past_its_own_insertion() {
    let mut h = alt_frame_app();
    run(&mut h, "review.trackChanges", json!({"value": true}));
    run(&mut h, "text.insert", json!({"text": "abXY"}));
    caret(&mut h, 2);
    alt_frame(&mut h, vec![surround(1, 1)]);
    assert_eq!(body(h.state()), "aY", "the own insertion `b` and the following `X` are erased");
    assert_eq!(h.state().session.sel.focus.off, 1, "the caret points after the stored deletion");
    assert_eq!(h.state().session.sel.anchor.off, 1);
}

/// Track Changes plus a refusal: the first side really removes this author's own insertion
/// from the stored paragraph, then a locked control refuses the second side. The whole
/// request is rolled back, including the shifted caret and undo stack.
#[test]
fn tracked_own_insertion_then_a_refused_side_rolls_the_whole_request_back() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "aXY"}));
    // Lock the following `X`; inserting `b` before it under Track Changes is this author's own
    // insertion, which a tracked delete removes from the stored text instead of marking.
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 2) };
    run(&mut h, "developer.richText", json!({"tag": "locked"}));
    run(&mut h, "developer.properties", json!({"control": "locked", "lockEdit": true}));
    run(&mut h, "review.trackChanges", json!({"value": true}));
    caret(&mut h, 1);
    run(&mut h, "text.insert", json!({"text": "b"}));
    assert_eq!(body(h.state()), "abXY");
    caret(&mut h, 2);
    let depth = h.state().session.undo_depth();
    let revisions = h.state().session.doc.revisions.len();
    let pending = wordcraft_doc::props::CharProps { bold: Some(true), ..Default::default() };
    h.state_mut().session.pending = Some(pending.clone());
    h.state_mut().session.goal_x = Some(123.0);
    alt_frame(&mut h, vec![surround(1, 1)]);
    assert_eq!(body(h.state()), "abXY", "the first side's own-insertion deletion is rolled back with the refused second side");
    assert_eq!(h.state().session.undo_depth(), depth, "the half-erasure leaves no undo step");
    assert_eq!(h.state().session.doc.revisions.len(), revisions, "revision metadata is restored too");
    assert!(h.state().canvas.ime_join_text.is_none(), "no rewrite is promised");
    assert!(resynced(&h), "the platform's stale surrounding snapshot is discarded");
    assert_eq!(h.state().session.sel.focus.off, 2, "the caret is back where the request found it");
    assert_eq!(h.state().session.pending, Some(pending), "pending formatting is restored");
    assert_eq!(h.state().session.goal_x, Some(123.0), "the Up/Down goal column is restored");
}

/// #6518 review: a refused `DeleteSurrounding` rolls back the formatting Backspace carried
/// over together with where it applies: typing at the restored caret still takes it, and a
/// caret moved away leaves it behind, as before the request.
#[test]
fn a_refused_delete_surrounding_keeps_formatting_backspace_carried() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "abc"}));
    // Wrap `c` in a rich-text control that refuses edits, and make `b` bold.
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 2), focus: wordcraft_doc::Pos::body(0, 3) };
    run(&mut h, "developer.richText", json!({"tag": "locked"}));
    run(&mut h, "developer.properties", json!({"control": "locked", "lockEdit": true}));
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 2) };
    run(&mut h, "format.bold", json!({"value": true}));
    caret(&mut h, 2);
    // Backspace the bold `b`: the caret, now after `a`, carries its bold.
    run(&mut h, "text.backspace", json!({}));
    assert_eq!(h.state().session.typing_props().bold, Some(true));
    alt_frame(&mut h, vec![surround(1, 1)]);
    assert_eq!(body(h.state()), "ac", "the request is refused and rolled back");
    assert_eq!(h.state().session.sel, wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1)));
    assert_eq!(h.state().session.typing_props().bold, Some(true), "the carried bold still applies at the restored caret");
    // It is still carried over by Backspace, not picked at the caret: a caret moved elsewhere
    // and back leaves it behind.
    h.state_mut().session.set_selection(wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 0)));
    h.state_mut().session.set_selection(wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1)));
    assert_ne!(h.state().session.typing_props().bold, Some(true), "the caret has been elsewhere");
}

/// Selects a body range, wraps it in a rich-text content control with `tag`, and sets its
/// placeholder.
fn wrap_in_control(h: &mut H, start: usize, end: usize, tag: &str, placeholder: &str) {
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, start), focus: wordcraft_doc::Pos::body(0, end) };
    run(h, "developer.richText", json!({"tag": tag}));
    run(h, "developer.properties", json!({"control": tag, "placeholder": placeholder}));
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
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "Name: AdaX"}));
    wrap_in_control(&mut h, 6, 9, "name", "Enter the full name");
    let r = control(h.state(), "name");
    let placeholder = r.control.placeholder_shown().to_string();
    assert!(placeholder.len() > "Ada".len(), "the fixture grows the paragraph: {placeholder:?}");
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(r.end.clone());
    let depth = h.state().session.undo_depth();

    alt_frame(&mut h, vec![surround(3, 1)]);
    assert_eq!(body(h.state()), format!("Name: {placeholder}"), "the following `X` is erased, not a placeholder character");
    let end = control(h.state(), "name").content_start().off + placeholder.len();
    let sel = &h.state().session.sel;
    assert_eq!((sel.anchor.off, sel.focus.off), (end, end), "the caret is at the placeholder's end, never inside it");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "both erased sides are one undo step");

    alt_frame(&mut h, vec![commit("N")]);
    assert_eq!(body(h.state()), "Name: N", "the commit replaces the restored placeholder");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "the replacement joins the deletion");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "Name: AdaX", "one Undo restores both erased sides and the replacement");
}

/// A placeholder shorter than the erased text also has to map the suffix and caret by the
/// signed delta; subtracting a removed-byte count alone describes only this direction.
#[test]
fn delete_surrounding_maps_a_shrinking_placeholder_and_after_text() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "Name: AdaX"}));
    wrap_in_control(&mut h, 6, 9, "name", "N");
    let r = control(h.state(), "name");
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(r.end.clone());
    let depth = h.state().session.undo_depth();

    alt_frame(&mut h, vec![surround(3, 1)]);
    assert_eq!(body(h.state()), "Name: N", "the shorter placeholder replaces `Ada`, and `X` still goes");
    let end = control(h.state(), "name").content_start().off + 1;
    let sel = &h.state().session.sel;
    assert_eq!((sel.anchor.off, sel.focus.off), (end, end));
    assert_eq!(h.state().session.undo_depth(), depth + 1);

    alt_frame(&mut h, vec![commit("Z")]);
    assert_eq!(body(h.state()), "Name: Z");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "Name: AdaX", "one Undo restores the original value and the following text");
}

/// The after side alone can empty a control and restore a longer placeholder. Its caret
/// must map to the restored placeholder's end, not to the old pre-deletion offset, and the
/// text beyond the after-side range stays put.
#[test]
fn delete_surrounding_maps_a_placeholder_restored_by_the_after_side() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "aXY"}));
    wrap_in_control(&mut h, 1, 2, "name", "Enter the value");
    let placeholder = control(h.state(), "name").control.placeholder_shown().to_string();
    assert!(placeholder.len() > 1);
    caret(&mut h, 1);
    let depth = h.state().session.undo_depth();

    alt_frame(&mut h, vec![surround(0, 1)]);
    assert_eq!(body(h.state()), format!("a{placeholder}Y"), "only the one after-side `X` is erased; `Y` is beyond the range");
    let end = control(h.state(), "name").content_start().off + placeholder.len();
    let sel = &h.state().session.sel;
    assert_eq!((sel.anchor.off, sel.focus.off), (end, end));
    assert_eq!(h.state().session.undo_depth(), depth + 1);

    alt_frame(&mut h, vec![commit("Q")]);
    assert_eq!(body(h.state()), "aQY", "the commit replaces the placeholder and leaves the following `Y`");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "aXY", "one Undo restores the emptied control, the following text and the commit");
}

/// A refused body request clears the locally painted pre-edit and every replacement join.
/// The earlier successful deletion stays, but the following commit gets its own Undo step.
#[test]
fn a_refused_body_delete_clears_preedit_and_joins() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "ab"}));
    wrap_in_control(&mut h, 1, 2, "locked", "b");
    run(&mut h, "developer.properties", json!({"control": "locked", "lockEdit": true}));
    caret(&mut h, 1);
    let depth = h.state().session.undo_depth();

    batch_frame(&mut h, vec![surround(1, 0), preedit("に"), surround(0, 1)]);
    assert_eq!(body(h.state()), "b", "the editable `a` is erased; the locked control's `b` is not");
    assert!(h.state().canvas.ime_preedit.is_empty(), "a refused request must not leave its pre-edit painted");
    assert_eq!(h.state().canvas.ime_preedit_range, None);
    assert!(h.state().canvas.ime_join_text.is_none(), "no replacement formatting is promised");
    assert!(interrupted(&h), "the platform's stale surroundings are interrupted");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "only the accepted first deletion has a step");

    complete_reset(&mut h);
    alt_frame(&mut h, vec![commit("X")]);
    assert_eq!(body(h.state()), "Xb");
    assert_eq!(h.state().session.undo_depth(), depth + 2, "the cleared join gives the commit its own checkpoint");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "b", "Undo takes back only the commit, not the earlier deletion");
}

/// A body deletion promises its join only to the immediate matching replacement. An ordinary
/// mutating command in between consumes the promise and keeps its own Undo checkpoint.
#[test]
fn body_delete_join_does_not_leak_to_an_intervening_command() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "abcd"}));
    caret(&mut h, 4);
    let base = h.state().session.undo_depth();

    alt_frame(&mut h, vec![surround(1, 0)]);
    assert_eq!(body(h.state()), "abc");
    assert_eq!(h.state().session.undo_depth(), base + 1, "the deletion is its own step");

    run(&mut h, "format.bold", json!({}));
    assert_eq!(body(h.state()), "abc", "the intervening command changes no text");
    assert_eq!(h.state().session.undo_depth(), base + 2, "an intervening ordinary command owns its checkpoint");

    alt_frame(&mut h, vec![commit("X")]);
    assert_eq!(body(h.state()), "abcX", "the replacement still applies");
    assert_eq!(h.state().session.undo_depth(), base + 3, "the replacement no longer joins across the ordinary command");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "abc", "Undo takes back only the commit, not the deletion or the Bold step");
}

/// A cross-block selection has no single surrounding snapshot. Refusing the request must
/// also interrupt the composition instead of leaving the platform's hidden text stale.
#[test]
fn cross_block_delete_surrounding_is_refused_and_interrupts() {
    let mut h = alt_frame_app();
    run(&mut h, "document.setText", json!({"text": "one\ntwo"}));
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(1, 1) };
    alt_frame(&mut h, vec![surround(1, 0)]);
    assert_eq!(body(h.state()), "one\ntwo", "nothing is deleted across blocks");
    assert!(resynced(&h), "the refused request interrupts the composition");
}

/// The linear text of the open equation.
fn linear(h: &mut H) -> String {
    run(h, "equation.get", json!({})).get("linear").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// An IME replacement inside an equation erases the counted characters with the equation's
/// commands; a byte count (which has no mapping into the tree) deletes nothing. A refusal
/// drops only the replacement immediately paired with it: the next `Commit`, or an adjacent
/// `Text` + `Commit`. A later key, pre-edit or direct commit in the same raw batch is fresh
/// input and applies; so does one in a later frame.
#[test]
fn ime_equation_byte_delete_drops_only_its_immediate_replacement() {
    let mut h = alt_frame_app();
    run(&mut h, "insert.equation", json!({"linear": "x+y"}));
    assert_eq!(linear(&mut h), "x+y");
    alt_frame(&mut h, vec![surround(1, 0)]);
    assert_eq!(linear(&mut h), "x+", "the character before the caret in the equation is erased");
    let base = linear(&mut h);

    // A direct commit in a later frame is fresh input, not the stale replacement.
    alt_frame(&mut h, vec![surround_bytes(1, 0)]);
    assert_eq!(linear(&mut h), base, "a byte count is not guessed into equation characters");
    assert!(resynced(&h), "the refused byte deletion interrupts the composition");
    alt_frame(&mut h, vec![commit("D")]);
    assert_eq!(linear(&mut h), format!("{base}D"), "a later direct commit applies without a pre-edit");

    // The immediately following commit belongs to the refused replacement and is dropped;
    // the next commit in the same raw batch is fresh input and applies.
    let after_pair = linear(&mut h);
    batch_frame(&mut h, vec![surround_bytes(1, 0), commit("Z"), commit("Y")]);
    assert_eq!(linear(&mut h), format!("{after_pair}Y"), "only the adjacent stale commit is dropped");
    let after_commit = linear(&mut h);

    // A platform that routes the replacement through `Text` before its `Commit` sends the
    // adjacent pair; both belong to the refused deletion, and a later commit still applies.
    batch_frame(&mut h, vec![surround_bytes(1, 0), egui::Event::Text("Q".into()), commit("Q2"), commit("R")]);
    assert_eq!(linear(&mut h), format!("{after_commit}R"), "the adjacent Text+Commit pair is stale, the next commit is not");

    // Any intervening event ends the pair: a key then its Text is ordinary fresh input.
    let after_commit = linear(&mut h);
    // Delete (at the equation's end it changes nothing) is a key the page reads; F20 is no editor input.
    let key = key_event(Key::Delete, true, Modifiers::NONE);
    // The framework takes a Delete in the frame right after a commit as the key that ended the composition
    // (`ime::GUARD_FRAMES`); this one is a later key, so a frame passes first.
    batch_frame(&mut h, Vec::new());
    batch_frame(&mut h, vec![surround_bytes(1, 0), key, egui::Event::Text("K".into())]);
    assert_eq!(linear(&mut h), format!("{after_commit}K"), "a key between the deletion and text clears the suppression");

    // A pre-edit likewise ends the stale pair; its commit is fresh.
    let after_commit = linear(&mut h);
    batch_frame(&mut h, vec![surround_bytes(1, 0), preedit("p"), commit("P")]);
    assert_eq!(linear(&mut h), format!("{after_commit}P"), "a fresh composition in the same batch is accepted");
}

/// An over-cap equation deletion is still a new surrounding request: it discards the
/// composition that was painted for the snapshot it refuses, and interrupts it.
#[test]
fn over_cap_equation_delete_clears_the_painted_preedit() {
    let mut h = alt_frame_app();
    run(&mut h, "insert.equation", json!({"linear": "abc"}));
    alt_frame(&mut h, vec![preedit("p")]);
    assert_eq!(h.state().canvas.ime_preedit, "p");
    alt_frame(&mut h, vec![surround(wordcraft_engine::MAX_EQUATION_DELETE_UNITS + 1, 0)]);
    assert_eq!(body(h.state()), "abc", "the over-cap request deletes nothing");
    assert!(h.state().canvas.ime_preedit.is_empty(), "the refused request's pre-edit is not left painted");
    assert_eq!(h.state().canvas.ime_preedit_range, None);
    assert!(interrupted(&h), "the platform's stale snapshot is interrupted");
}

/// An over-cap equation deletion also clears a previous equation deletion's promised join,
/// so the next commit cannot silently merge into that earlier undo step.
#[test]
fn over_cap_equation_delete_clears_the_previous_replacement_join() {
    let mut h = alt_frame_app();
    run(&mut h, "insert.equation", json!({"linear": "abcd"}));
    alt_frame(&mut h, vec![surround(1, 0), surround(wordcraft_engine::MAX_EQUATION_DELETE_UNITS + 1, 0)]);
    assert_eq!(body(h.state()), "abc", "the first, valid deletion stands");
    assert!(resynced(&h), "the over-cap request interrupts the composition");
    let depth = h.state().session.undo_depth();
    alt_frame(&mut h, vec![commit("X")]);
    assert_eq!(body(h.state()), "abcX");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "the cleared join gives the commit its own undo step");
}

/// The transactional equation delete and the following commit are one undo step, like the
/// body's `DeleteSurrounding` plus its replacement text.
#[test]
fn ime_equation_delete_and_commit_are_one_undo_step() {
    let mut h = alt_frame_app();
    run(&mut h, "insert.equation", json!({"linear": "abcd"}));
    alt_frame(&mut h, vec![surround(1, 0), commit("X")]);
    assert_eq!(body(h.state()), "abcX");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "abcd", "one Undo restores the deleted unit and the commit together");
}

/// An equation deletion promises a join only to its immediate replacement. An unrelated
/// command in between consumes the promise without joining, so it gets its own undo step
/// instead of being merged into the deletion.
#[test]
fn ime_equation_delete_join_does_not_leak_to_an_unrelated_command() {
    let mut h = alt_frame_app();
    run(&mut h, "insert.equation", json!({"linear": "abcd"}));
    let base = h.state().session.undo_depth();
    alt_frame(&mut h, vec![surround(1, 0)]);
    assert_eq!(body(h.state()), "abc");
    assert_eq!(h.state().session.undo_depth(), base + 1, "the deletion is its own undo step");

    run(&mut h, "equation.justify", json!({"jc": "right"}));
    assert_eq!(h.state().session.undo_depth(), base + 2, "an unrelated command keeps its own checkpoint");

    alt_frame(&mut h, vec![commit("X")]);
    assert_eq!(body(h.state()), "abcX");
    assert_eq!(h.state().session.undo_depth(), base + 3, "the replacement no longer joins across the unrelated command");

    run(&mut h, "edit.undo", json!({}));
    assert_eq!(body(h.state()), "abc", "Undo takes back only the commit, not the deletion and the unrelated command");
}

/// Hostile pre-edit and commit sizes are refused before they reach the canvas or the model.
#[test]
fn oversized_ime_events_are_ignored() {
    let mut h = alt_frame_app();
    let huge = "a".repeat(MAX_IME_PREEDIT_BYTES + 1);
    alt_frame(&mut h, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: huge, active_range_chars: Some(0..1) })]);
    assert!(h.state().canvas.ime_preedit.is_empty(), "an oversized pre-edit is not painted");
    assert_eq!(h.state().canvas.ime_preedit_range, None);
    let huge = "b".repeat(MAX_IME_COMMIT_BYTES + 1);
    alt_frame(&mut h, vec![commit(&huge)]);
    assert_eq!(body(h.state()), "", "an oversized commit is not inserted");
    // The equation cap is lower than the byte cap: an over-cap commit is refused before the
    // command, with no prefix committed and the composition interrupted.
    run(&mut h, "insert.equation", json!({}));
    let long = "c".repeat(wordcraft_engine::MAX_EQUATION_TYPE_CHARS + 1);
    alt_frame(&mut h, vec![commit(&long)]);
    assert_eq!(body(h.state()), "", "no equation prefix is committed");
    assert!(resynced(&h), "the refused commit starts the composition afresh");
}

#[test]
fn typing_with_smart_art_selected_types_into_an_item() {
    let mut h = alt_frame_app();
    run(&mut h, "insert.smartArt", json!({"layout": "matrix", "items": ["Ada", "Bea"]}));
    for t in ["x", "y"] {
        alt_frame(&mut h, vec![egui::Event::Text(t.into())]);
    }
    let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&h.state().session).expect("the graphic stays");
    assert_eq!(g.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["Adaxy", "Bea"]);
    assert!(h.state().session.view.smart_art_pane, "the Text Pane opens");
    run(&mut h, "edit.undo", json!({}));
    let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&h.state().session).expect("still there");
    assert_eq!(g.items[0].text, "Ada", "one undo step for the typing");
}

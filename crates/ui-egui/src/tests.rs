use super::*;
use egui_kittest::kittest::Queryable;
// PLAN(app-hub): assumed facade paths for the host fakes and the automation channel (S-L103, S-L150).
use craft::automation::control::{ControlRequest, Reply};
use craft::host::{PickId, PickKind, PickRequest, Picked, ScriptedFiles};
use craft::platform::clock::monotonic_ms;
use craft::platform::default_app::{DefaultApp, DefaultAppError};
use craft::platform::open::{DropTarget, DroppedFile, FileIntake};
use craft::ui::shell::ScreenMode;
use std::sync::Arc;

mod perf;
// The new test files are children of `tests` so they reach its private helpers (`app`, `harness`, ...).
// They are not named `chrome`, `dialogs` or `ribbon`: inside `tests` those names must keep meaning the
// crate's own modules (`dialogs::Dialog`, `chrome::qat_enabled`, ...), and `control.rs` still has an
// inline `mod tests`.
#[path = "tests/chrome.rs"]
mod chrome_tests;
#[path = "control/tests.rs"]
mod control_tests;
#[path = "tests/dialogs.rs"]
mod dialogs_tests;
#[path = "tests/ribbon.rs"]
mod ribbon_tests;

fn mod_key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }
}

/// #5793: Save As suggests the first words for a document that was never saved and has no title
/// of its own (the first paragraph with text, past empty ones); a title, a file name or a path
/// still name it.
#[test]
fn save_as_suggests_the_first_words_of_a_new_document() {
    let mut app = app();
    assert_eq!(app.default_save_name(), format!("{}.docx", app.title_stem()), "an empty document");
    for c in [
        json!(["text.newParagraph", {}]),
        json!(["text.insert", {"text": "Minutes of the board meeting. Present: all"}]),
        json!(["text.newParagraph", {}]),
        json!(["text.insert", {"text": "Second paragraph"}]),
    ] {
        app.session.run(c[0].as_str().unwrap(), &c[1]).unwrap();
    }
    assert_eq!(app.default_save_name(), "Minutes of the board meeting.docx");
    assert_eq!(
        app.title_stem(),
        crate::documents::untitled_title(&app.session.resolved_filenames().0).unwrap_or_else(|| "Document1".into()),
        "the window title stays Document1"
    );
    app.session.doc.core.title = "Board minutes".into();
    assert_eq!(app.default_save_name(), "Board minutes.docx", "a title wins");
    app.session.doc.core.title.clear();
    app.session.path = Some(std::path::PathBuf::from("/tmp/kept.docm"));
    assert_eq!(app.default_save_name(), "kept.docm", "a saved file keeps its name and format");
}

/// #3634: every source of the title (path, file name, document title) drops NUL.
#[test]
fn title_stem_omits_nul_from_each_title_source() {
    let mut app = app();
    let fallback = app.title_stem();
    for title in ["Résumé\u{0} notes", "\u{0}"] {
        let expected = if title == "\u{0}" { fallback.as_str() } else { "Résumé notes" };
        app.session.doc.core.title = title.into();
        assert_eq!(app.title_stem(), expected);
        app.session.file_name = Some(title.into());
        assert_eq!(app.title_stem(), expected);
        app.session.path = Some(std::path::PathBuf::from(format!("{title}.docx")));
        assert_eq!(app.title_stem(), expected);
        app.session.path = None;
        app.session.file_name = None;
    }
}

/// A harness on a document with text whose page has the keyboard.
fn canvas_harness() -> egui_kittest::Harness<'static, WordApp> {
    let mut a = app();
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    h.run_steps(3);
    assert!(h.state().canvas.focused(&h.ctx), "the page has the keyboard");
    h
}

/// Ctrl+F1 pressed on the page, through the app's key handling.
#[cfg(not(target_os = "macos"))]
fn press_ctrl_f1(h: &mut egui_kittest::Harness<'static, WordApp>) {
    let modifiers = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
    h.event(egui::Event::Key { key: egui::Key::F1, physical_key: None, pressed: true, repeat: false, modifiers });
    h.event(egui::Event::Key { key: egui::Key::F1, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(3);
}

/// A harness over the app, as the tables below drive it.
type Rig = egui_kittest::Harness<'static, WordApp>;

/// One key press with `modifiers`, then two frames: the runtime fires the chord's command in the first, and the app
/// runs it before the second (`raw_input_hook`), as on the desktop.
fn press(h: &mut Rig, key: egui::Key, modifiers: egui::Modifiers) {
    h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.step();
    h.step();
}

/// The style of the first paragraph.
fn first_style(h: &Rig) -> Option<String> {
    h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.style.clone())
}

/// The shown selection's bold state.
fn bold_state(h: &mut Rig) -> Option<bool> {
    h.state_mut().run("format.state", json!({})).unwrap().get("bold").and_then(Value::as_bool)
}

/// A harness on an empty document with the first frames run.
fn blank_rig() -> Rig {
    let mut h = harness(app());
    h.run_steps(6);
    h
}

fn row_heading_shortcut() {
    let mut h = blank_rig();
    h.event(egui::Event::Text("Kilns".into()));
    h.step();
    press(&mut h, egui::Key::Num1, egui::Modifiers::COMMAND | egui::Modifiers::ALT);
    assert_eq!(first_style(&h).as_deref(), Some("Heading1"));
    // Alt+F7 can come with key text on some platforms; none is typed.
    // The platform sends the key's text in the frame of the press, which is where the runtime withholds it.
    // Both in one frame's input: kittest's `event` runs a frame for each queued event.
    h.input_mut().events.extend([
        egui::Event::Key { key: egui::Key::F7, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT },
        egui::Event::Text("x".into()),
    ]);
    h.step();
    h.step();
    assert_eq!(body_text(h.state()), "Kilns", "an Alt shortcut types nothing");
}

fn row_ctrl_q_resets_the_paragraph() {
    let mut h = blank_rig();
    h.event(egui::Event::Text("Kilns".into()));
    h.step();
    h.state_mut().run("para.alignCenter", json!({})).unwrap();
    h.step();
    let align = |h: &Rig| h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.align);
    assert!(align(&h).is_some());
    // Off the Mac, Ctrl is the command key: egui sets both `ctrl` and `command`.
    let ctrl_q = if cfg!(target_os = "macos") { egui::Modifiers::CTRL } else { egui::Modifiers::CTRL | egui::Modifiers::COMMAND };
    press(&mut h, egui::Key::Q, ctrl_q);
    assert_eq!(align(&h), None, "Ctrl+Q resets the paragraph formatting");
    press(&mut h, egui::Key::Q, egui::Modifiers::CTRL | egui::Modifiers::COMMAND);
    assert!(!h.state().quit_requested && prompt(h.state()).is_none(), "Ctrl+Q doesn't close the window");
    // ⌘Q is the runtime's (S-L332): the platform's quit reaches the app as a close of the root window, which the app's
    // close decision answers. The old app read ⌘Q as a key press on every host.
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.step();
    h.step();
    assert_eq!(prompt(h.state()), Some("saveChanges"), "⌘Q still quits, asking about the typed text first");
}

#[cfg(not(target_os = "macos"))]
fn row_ctrl_alt_q_types_at() {
    let mut h = blank_rig();
    let german = egui::Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: true };
    press(&mut h, egui::Key::Q, german);
    assert!(!h.state().quit_requested && prompt(h.state()).is_none(), "Ctrl+Alt+Q does not close the window");
    h.event(egui::Event::ModifiersChanged(german));
    h.event(egui::Event::Text("@".into()));
    h.step();
    assert_eq!(body_text(h.state()), "@", "Ctrl+Alt+Q types the @ character");
}

fn row_mod_minus_does_not_zoom_the_window() {
    let mut h = blank_rig();
    h.event(mod_key(egui::Key::Minus));
    h.step();
    h.event(mod_key(egui::Key::Minus));
    h.step();
    assert_eq!(h.ctx.zoom_factor(), 1.0, "Mod+- is the optional hyphen, not window zoom");
}

#[cfg(not(target_os = "macos"))]
fn row_unbound_ctrl_f1_collapses_the_ribbon() {
    let mut h = canvas_harness();
    assert!(!h.state().ui.ribbon_collapsed);
    press_ctrl_f1(&mut h);
    assert!(h.state().ui.ribbon_collapsed, "Ctrl+F1 collapses the ribbon");
    assert!(!h.state().session.view.focus_mode);
    press_ctrl_f1(&mut h);
    assert!(!h.state().ui.ribbon_collapsed, "and brings it back");
}

#[cfg(not(target_os = "macos"))]
fn row_bound_ctrl_f1_runs_the_bound_command() {
    let mut h = canvas_harness();
    // PLAN(shell-w7): sh.A11 `Session::keymap_mut` over `Overrides::insert`.
    h.state_mut().session.keymap_mut().insert("view.focus", vec!["Mod+F1".into()]);
    press_ctrl_f1(&mut h);
    assert!(h.state().session.view.focus_mode, "the bound command ran");
    assert!(!h.state().ui.ribbon_collapsed, "the ribbon is unchanged");
}

fn row_mod_shift_c_then_v_pastes_formatting_not_text() {
    // A native event carries ⌘ (mac_cmd and command) on macOS, Ctrl (ctrl and command)
    // elsewhere; COMMAND+SHIFT is the logical chord both platforms resolve.
    let native = if cfg!(target_os = "macos") {
        egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
    } else {
        egui::Modifiers::CTRL | egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
    };
    for modifiers in [egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, native] {
        let mut a = app();
        a.run("text.insert", json!({"text": "Bold plain"})).unwrap();
        a.run("select.text", json!({"text": "Bold"})).unwrap();
        a.run("format.bold", json!({})).unwrap();
        a.run("select.text", json!({"text": "plain"})).unwrap();
        a.run("edit.copy", json!({})).unwrap();
        let entries = a.session.clip_history.len();
        a.run("select.text", json!({"text": "Bold"})).unwrap();
        let mut h = harness(a);
        h.run_steps(6);
        assert_eq!(bold_state(&mut h), Some(true), "the source selection is bold");
        press(&mut h, egui::Key::C, modifiers);
        assert!(h.state().session.copied_format.is_some(), "{modifiers:?}: Copy Formatting picked up the formatting");
        assert!(h.state().session.painter.is_none(), "{modifiers:?}: it doesn't arm the Format Painter brush");
        assert_eq!(h.state().session.clipboard_text, "plain", "{modifiers:?}: the clipboard text is kept");
        assert_eq!(body_text(h.state()), "Bold plain", "{modifiers:?}: the text is untouched");
        h.state_mut().run("select.text", json!({"text": "plain"})).unwrap();
        assert_eq!(bold_state(&mut h), Some(false));
        press(&mut h, egui::Key::V, modifiers);
        assert_eq!(bold_state(&mut h), Some(true), "{modifiers:?}: Paste Formatting made the target bold");
        assert_eq!(body_text(h.state()), "Bold plain", "{modifiers:?}: it pasted no text");
        assert_eq!(h.state().session.clip_history.len(), entries, "{modifiers:?}: clipboard history is kept");
    }
}

fn row_mod_shift_v_without_copied_formatting_pastes_text_only() {
    let mut a = app();
    a.run("text.insert", json!({"text": "Bold plain"})).unwrap();
    a.run("select.text", json!({"text": "Bold"})).unwrap();
    a.run("format.bold", json!({})).unwrap();
    a.run("edit.copy", json!({})).unwrap();
    a.run("select.text", json!({"text": "plain"})).unwrap();
    // The destination has formatting of its own, so a paste that dropped it would show.
    a.run("format.italic", json!({})).unwrap();
    assert!(a.session.painter.is_none());
    let mut h = harness(a);
    h.run_steps(6);
    press(&mut h, egui::Key::V, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!(body_text(h.state()), "Bold Bold", "the clipboard text replaced the selection");
    assert_ne!(bold_state(&mut h), Some(true), "the text took the destination's formatting");
    let italic = h.state_mut().run("format.state", json!({})).unwrap().get("italic").and_then(Value::as_bool);
    assert_eq!(italic, Some(true), "and kept the destination's italic");
}

fn row_shift_delete_cuts_and_shift_backspace_erases() {
    let mut h = blank_rig();
    h.event(egui::Event::Text("Kilns fire clay".into()));
    h.step();
    press(&mut h, egui::Key::Backspace, egui::Modifiers::SHIFT);
    assert_eq!(body_text(h.state()), "Kilns fire cla");
    press(&mut h, egui::Key::Home, egui::Modifiers::NONE);
    press(&mut h, egui::Key::Delete, egui::Modifiers::SHIFT);
    assert_eq!(body_text(h.state()), "Kilns fire cla", "no selection: nothing to cut");
    h.state_mut().run("select.text", json!({"text": "fire "})).unwrap();
    press(&mut h, egui::Key::Delete, egui::Modifiers::SHIFT);
    assert_eq!(body_text(h.state()), "Kilns cla");
    assert_eq!(h.state().session.clipboard_text, "fire ", "cut, not just deleted");
}

#[cfg(not(target_os = "macos"))]
fn row_mod_shift_0_resets_the_interface_size_unless_bound() {
    let chord = egui::Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };
    let mut h = blank_rig();
    h.state_mut().run("ui.interfaceSize", json!({"percent": 300})).unwrap();
    // AltGr+0 types `}` on a German layout: it neither resets nor is swallowed.
    let altgr = egui::Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: true };
    h.event(egui::Event::Key { key: egui::Key::Num0, physical_key: None, pressed: true, repeat: false, modifiers: altgr });
    h.event(egui::Event::ModifiersChanged(altgr));
    h.event(egui::Event::Text("}".into()));
    h.step();
    assert_eq!(h.state().ui.interface_size.percent(), Some(300), "AltGr+0 doesn't reset");
    assert_eq!(body_text(h.state()), "}", "AltGr+0 types the character");
    h.event(egui::Event::Key { key: egui::Key::Num0, physical_key: Some(egui::Key::Num0), pressed: true, repeat: false, modifiers: chord });
    h.step();
    h.step();
    assert!(h.state().ui.interface_size.is_auto(), "Ctrl+Shift+0 returns to Auto");
    // A binding the user made for the chord wins.
    h.state_mut().run("ui.interfaceSize", json!({"percent": 300})).unwrap();
    // PLAN(shell-w7): sh.A11 `Session::keymap_mut` over `Overrides::insert`.
    h.state_mut().session.keymap_mut().insert("view.focus", vec!["Mod+Shift+0".into()]);
    h.event(egui::Event::Key { key: egui::Key::Num0, physical_key: Some(egui::Key::Num0), pressed: true, repeat: false, modifiers: chord });
    h.step();
    h.step();
    assert_eq!(h.state().ui.interface_size.percent(), Some(300), "the size stays");
    assert!(h.state().session.view.focus_mode, "the user's binding runs");
}

/// Where Shift+0 types `=` (German, Swiss and Nordic layouts) the press is Ctrl+Shift+=, Superscript,
/// and the interface size stays.
#[cfg(not(target_os = "macos"))]
fn row_mod_shift_equals_stays_superscript() {
    let mut a = app();
    a.run("document.setText", json!({"text": "x"})).unwrap();
    a.run("select.all", json!({})).unwrap();
    a.run("ui.interfaceSize", json!({"percent": 300})).unwrap();
    let mut h = harness(a);
    h.run_steps(6);
    let chord = egui::Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };
    h.event(egui::Event::Key { key: egui::Key::Equals, physical_key: Some(egui::Key::Num0), pressed: true, repeat: false, modifiers: chord });
    h.step();
    // The chord fires its command in that pass; the app runs it before the next pass (`raw_input_hook`).
    h.step();
    assert_eq!(h.state().ui.interface_size.percent(), Some(300), "Ctrl+Shift+= doesn't reset");
    let pos = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
    let props = h.state().session.doc.para_at(&pos).map(|p| p.props_of_char(0).clone()).unwrap_or_default();
    assert_eq!(props.vert_align, Some(wordcraft_doc::props::VertAlign::Superscript), "Ctrl+Shift+= runs Superscript");
}

/// The default chords reach their commands through the app's catalog and the board, and each does
/// what Word's chord does: the effect is asserted, not the key name.
#[test]
fn default_chords_fire_their_commands() {
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    let mut rows: Vec<(&str, fn())> = vec![
        ("Ctrl+Alt+1 applies Heading 1 and types nothing", row_heading_shortcut),
        ("Ctrl+Q resets the paragraph and does not close", row_ctrl_q_resets_the_paragraph),
        ("Mod+- does not zoom the window", row_mod_minus_does_not_zoom_the_window),
        ("Mod+Shift+C then Mod+Shift+V pastes formatting, not text", row_mod_shift_c_then_v_pastes_formatting_not_text),
        ("Mod+Shift+V without copied formatting pastes text only", row_mod_shift_v_without_copied_formatting_pastes_text_only),
        ("Shift+Delete cuts, Shift+Backspace erases", row_shift_delete_cuts_and_shift_backspace_erases),
    ];
    #[cfg(not(target_os = "macos"))]
    rows.extend([
        ("Ctrl+Alt+Q types @", row_ctrl_alt_q_types_at as fn()),
        ("unbound Ctrl+F1 collapses the ribbon, a second brings it back", row_unbound_ctrl_f1_collapses_the_ribbon),
        ("a bound Ctrl+F1 runs the bound command", row_bound_ctrl_f1_runs_the_bound_command),
        ("Mod+Shift+0 resets the interface size unless bound", row_mod_shift_0_resets_the_interface_size_unless_bound),
        ("Mod+Shift+= stays Superscript", row_mod_shift_equals_stays_superscript),
    ]);
    for (name, row) in rows {
        eprintln!("row: {name}");
        row();
    }
}

/// The key of the dialog a command opened, read off its reply.
fn pending_of(reply: &Value) -> Option<&str> {
    reply.get("pending").and_then(Value::as_str)
}

/// What a row's command does for a script, which is never shown a dialog.
enum ForScripts {
    /// The command fails for lack of parameters.
    Errors,
    /// The command answers with its data.
    Data,
}

/// One row of `input_dialog_opens_for_users_and_never_for_scripts`.
struct InputRow {
    command: &'static str,
    params: Value,
    setup: Vec<(&'static str, Value)>,
    dialog: &'static str,
    scripts: ForScripts,
    /// Parameters that apply the command without asking.
    applies_with: Option<Value>,
}

/// A command that needs input opens its dialog when a user runs it and never when a script does.
/// One mapping rule: command and parameters → dialog key.
#[test]
fn input_dialog_opens_for_users_and_never_for_scripts() {
    let csv = json!({"csv": "First Name,City\nAda,London"});
    let rows = vec![
        InputRow {
            command: "insert.icon",
            params: json!({}),
            setup: vec![],
            dialog: "icons",
            scripts: ForScripts::Data,
            applies_with: Some(json!({"id": "sun", "size": 36})),
        },
        InputRow {
            command: "format.enclose",
            params: json!({}),
            setup: vec![("text.insert", json!({"text": "a 字 b"})), ("select.text", json!({"text": "字"}))],
            dialog: "enclose",
            // `format.enclose` needs its `shape` (`engine/src/cmd/format.rs`, as in production): no data to list.
            scripts: ForScripts::Errors,
            applies_with: Some(json!({"shape": "square", "style": "enlarge", "text": "字"})),
        },
        InputRow {
            command: "insert.field",
            params: json!({}),
            setup: vec![],
            dialog: "field",
            scripts: ForScripts::Errors,
            applies_with: Some(json!({"instr": "PAGE"})),
        },
        InputRow {
            command: "fields.setCode",
            params: json!({}),
            setup: vec![("insert.field", json!({"instr": "PAGE"})), ("caret.docStart", json!({}))],
            dialog: "editFieldCode",
            scripts: ForScripts::Errors,
            applies_with: None,
        },
        InputRow {
            command: "mailings.rules",
            params: json!({"rule": "IF"}),
            setup: vec![("mailings.recipients", csv.clone())],
            dialog: "ruleIf",
            scripts: ForScripts::Data,
            applies_with: None,
        },
        InputRow {
            command: "mailings.matchFields",
            params: json!({}),
            setup: vec![("mailings.recipients", csv.clone())],
            dialog: "matchFields",
            scripts: ForScripts::Data,
            applies_with: None,
        },
        InputRow {
            command: "mailings.checkErrors",
            params: json!({}),
            setup: vec![("mailings.recipients", csv)],
            dialog: "checkErrors",
            scripts: ForScripts::Data,
            applies_with: None,
        },
    ];
    // What a script gets from `row`'s command, on the same setup.
    let script_reply = |row: &InputRow| {
        let mut script = app();
        for (command, params) in &row.setup {
            script.run(command, params.clone()).unwrap();
        }
        script.execute(row.command, row.params.clone()).unwrap()
    };
    for row in rows {
        let setup = |a: &mut WordApp| {
            for (command, params) in &row.setup {
                a.run(command, params.clone()).unwrap();
            }
        };
        let mut user = app();
        setup(&mut user);
        let reply = user.run(row.command, row.params.clone()).unwrap();
        // Match Fields and Check for Errors ask nothing: they report what they found, so the user's reply is the
        // command's data, as a script's is, and the dialog shows it (production's `Dialog::report`).
        if matches!(row.command, "mailings.matchFields" | "mailings.checkErrors") {
            assert_eq!(pending_of(&reply), None, "{}: a report asks nothing", row.command);
            assert_eq!(reply, script_reply(&row), "{}: the user gets the data a script gets", row.command);
            if row.dialog == "matchFields" {
                assert!(
                    user.dialog_host.open_dialogs().into_iter().any(|d| d.key.as_deref() == Some(row.dialog)),
                    "{}: its report shows",
                    row.command
                );
            }
        } else {
            assert_eq!(pending_of(&reply), Some(row.dialog), "{}: a user is asked", row.command);
        }
        // PLAN(dialogs-w1): the app's `dialog_host` field is the window's `DialogHost`; `is_empty` is dg.F3's sibling.
        assert!(!user.dialog_host.is_empty(), "{}: the dialog is open", row.command);
        if let Some(params) = &row.applies_with {
            let mut applied = app();
            setup(&mut applied);
            let reply = applied.run(row.command, params.clone()).unwrap();
            assert_eq!(pending_of(&reply), None, "{}: parameters apply it without asking", row.command);
            assert!(applied.dialog_host.is_empty(), "{}: and open nothing", row.command);
        }
        let mut script = app();
        setup(&mut script);
        match row.scripts {
            ForScripts::Errors => assert!(script.execute(row.command, row.params.clone()).is_err(), "{}: a script gets an error", row.command),
            ForScripts::Data => assert!(script.execute(row.command, row.params.clone()).is_ok(), "{}: a script gets its data", row.command),
        }
        assert!(script.dialog_host.is_empty(), "{}: a script never opens a dialog", row.command);
    }
}

/// The CJK UI font a Japanese file name asks for is loaded and draws the name, also in a build
/// without embedded CJK faces: the caption's glyphs exist two frames after the name is set.
#[test]
fn a_japanese_file_name_is_drawn_in_the_caption() {
    let mut a = app();
    a.session.path = Some("日本語.docx".into());
    let mut h = harness(a);
    h.run_steps(2);
    // PLAN(shell-w11): the installed `ui_fonts` stack answers `has_glyphs` for the proportional face.
    let drawn = h.ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(13.0), "日本語"));
    // macOS and Windows always have a CJK font and CI runs these tests on both, so there a missing
    // font fails instead of hiding a regression.
    assert!(drawn || cfg!(not(any(target_os = "macos", windows))), "the caption draws the name");
}

/// The app opens its window and shows it for a few frames at the size of a laptop screen: the whole
/// root (shell, slots, body, dialog host) runs in a debug build, where a debug assertion on the
/// first frame fires that release builds and the components' own trials never reach.
#[test]
fn the_app_window_shows_for_a_few_frames() {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app());
    h.run_steps(3);
    assert!(h.state().has_document(), "the window opened on a document");
    assert!(!h.state().quit_requested);
}

fn app() -> WordApp {
    app_with_files(Arc::new(ScriptedFiles::new()))
}

/// An app over `session` whose host answers file requests from `files`.
fn app_over(session: Session, files: Arc<ScriptedFiles>) -> WordApp {
    let host = craft::host::Host::builder(crate::APP).files(files).build();
    WordApp::new(session, Services::default()).with_host(Arc::new(host))
}

/// A new-document app whose host answers file requests from `files`.
fn app_with_files(files: Arc<ScriptedFiles>) -> WordApp {
    app_over(Session::new(wordcraft_doc::Document::new()), files)
}

/// The next picker answer of `files`: the path chosen, or a cancel.
fn script_pick(files: &ScriptedFiles, picked: Option<String>) {
    files.queue(match picked {
        Some(p) => Picked::Paths(vec![std::path::PathBuf::from(p)]),
        None => Picked::Cancelled,
    });
}

/// The status bar's text, while it shows (sh.A3).
fn status_text(a: &WordApp) -> Option<&str> {
    a.status.as_ref().map(|s| s.text.as_str())
}

/// The key and fields of the dialog on top, from the dialog host.
fn open_dialog(a: &WordApp) -> Option<(String, Value)> {
    a.dialog_host.open_dialogs().into_iter().next_back().map(|d| (d.key.unwrap_or_default(), d.fields.unwrap_or_default()))
}

/// Closes every open dialog.
fn close_dialogs(a: &mut WordApp, ctx: &egui::Context) {
    crate::dialogs::close_all(a, ctx);
}

/// Offers a picture on the installed clipboard for the next paste.
///
/// The browser delivers a paste as a late offer that `arrived` reports for one frame (sh.A18); the default test
/// clipboard has no such queue, so the paste harness installs [`LatePaste`] and this queues the picture on it.
fn offer_paste(_ctx: &egui::Context, png: Vec<u8>) {
    use craft::platform::clipboard::{Flavor, RGBA8};
    let rgba = image::load_from_memory(&png).unwrap().to_rgba8();
    let mut data = Vec::new();
    data.extend_from_slice(&rgba.width().to_le_bytes());
    data.extend_from_slice(&rgba.height().to_le_bytes());
    data.extend_from_slice(rgba.as_raw());
    LATE_PASTES.with(|q| q.borrow_mut().push(Flavor::new(RGBA8, data)));
}

thread_local! {
    /// Pictures offered by [`offer_paste`], each taken as one late paste by [`LatePaste`].
    static LATE_PASTES: std::cell::RefCell<Vec<craft::platform::clipboard::Flavor>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// An in-memory clipboard that, like the event clipboard of the web build, takes a queued paste as one late arrival.
#[derive(Default)]
struct LatePaste(craft::platform::clipboard::MemoryClipboard);

impl craft::platform::clipboard::ClipboardBackend for LatePaste {
    fn write(&mut self, flavors: &[craft::platform::clipboard::Flavor]) -> Result<usize, craft::platform::clipboard::ClipboardError> {
        self.0.write(flavors)
    }

    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<craft::platform::clipboard::Flavor>, craft::platform::clipboard::ClipboardError> {
        self.0.read(mimes, limit)
    }

    fn read_image(&mut self, limit: u64) -> Result<Option<craft::platform::clipboard::ClipboardImage>, craft::platform::clipboard::ClipboardError> {
        self.0.read_image(limit)
    }

    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<std::path::PathBuf>>, craft::platform::clipboard::ClipboardError> {
        self.0.read_files(limit)
    }

    fn has(&mut self, mimes: &[&str]) -> bool {
        self.0.has(mimes)
    }

    fn take_arrived(&mut self) -> bool {
        let next = LATE_PASTES.with(|q| {
            let mut q = q.borrow_mut();
            (!q.is_empty()).then(|| q.remove(0))
        });
        match next {
            Some(flavor) => self.0.write(&[flavor]).is_ok(),
            None => false,
        }
    }
}

/// Files handed to the app as a drop, through its intake.
fn drop_files(a: &mut WordApp, files: Vec<DroppedFile>) {
    let (intake, sender) = FileIntake::new(craft::host::Wake::none());
    a.services.intake = Some(intake);
    sender.dropped(files, DropTarget::Elsewhere);
}

/// An in-memory dropped file.
fn dropped_bytes(name: &str, bytes: Vec<u8>) -> DroppedFile {
    DroppedFile { path: None, name: name.into(), bytes: Some(bytes.into()) }
}

/// #2184: the Linux display server is a UI command, kept in `ui.json` for the desktop app to
/// read at the next launch; junk is refused, and an unknown saved value reads as Automatic.
#[test]
fn display_server_is_a_ui_command_and_survives_a_restart() {
    let mut a = app();
    assert_eq!(a.run("ui.displayServer", json!({})).unwrap(), json!({"displayServer": "auto"}), "automatic by default; no value reads it");
    assert_eq!(a.run("ui.displayServer", json!({"value": " X11 "})).unwrap(), json!({"displayServer": "x11"}));
    for junk in [json!({"value": "mir"}), json!({"value": 1}), json!({"value": null}), json!({"value": ["wayland"]})] {
        assert!(a.run("ui.displayServer", junk).is_err());
    }
    assert_eq!(a.ui.display_server, DisplayServer::X11, "a rejected value keeps the setting");
    let mut restarted = app();
    restarted.apply_prefs(a.prefs());
    assert_eq!(restarted.ui.display_server, DisplayServer::X11, "the saved value comes back");
    for old in [r#"{"tab":"Insert"}"#, r#"{"displayServer":"mir"}"#, r#"{"displayServer":7}"#] {
        // PLAN(shell-w13): sh.A12 `legacy_ui_json` reads the old bytes into `Prefs`.
        let (prefs, _) = crate::prefs::legacy_ui_json(old.as_bytes()).unwrap();
        let mut b = app();
        b.apply_prefs(prefs);
        assert_eq!(b.ui.display_server, DisplayServer::Auto, "{old}");
    }
}

fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a)
}

/// #1908: with no document open the Home ribbon's split buttons grey out like the plain
/// buttons beside them, and their drop-down arrow opens no menu.
#[test]
fn split_buttons_are_disabled_with_no_document() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut a = app();
    a.start_screen();
    a.run("ui.backstage", json!({"value": false})).unwrap();
    assert!(!a.has_document());
    let mut h = harness(a);
    h.run_steps(4);
    for tip in ["Underline", "Text Highlight Color", "Font Color", "Bullets", "Numbering", "Shading", "Borders"] {
        // A split button names both its parts (the action and its arrow) by the same label.
        let nodes: Vec<_> = h.query_all_by_role_and_label(Role::Button, tip).collect();
        assert!(!nodes.is_empty() && nodes.iter().all(|n| n.accesskit_node().is_disabled()), "{tip} is greyed out with no document");
    }
    let arrow = h
        .query_all_by_role_and_label(Role::Button, "Underline")
        .map(|n| n.rect())
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .map(|r| r.center())
        .expect("the Underline split button shows");
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: arrow, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    h.run_steps(3);
    assert!(!egui::Popup::is_any_open(&h.ctx), "the disabled split button's arrow opens no menu");
}

/// #1103: a file that fails to open names the file and the real cause on the status bar.
#[test]
fn a_file_that_wont_open_says_which_file_and_why() {
    let dir = scratch("open-failure");
    let bad = dir.join("broken.docx");
    std::fs::write(&bad, b"PK\x03\x04 this is not really a zip").unwrap();
    let mut a = app();
    let path = bad.to_string_lossy().to_string();
    let error = a.run("file.open", json!({"path": path})).unwrap_err();
    assert!(error.contains("zip") || error.contains("Word"), "the engine names the cause: {error}");
    let msg = a.report_open_failure(&path, &error);
    assert!(msg.contains(&path) && msg.contains(&error), "{msg}");
    assert_eq!(status_text(&a), Some(msg.as_str()));

    let missing = dir.join("gone.docx").to_string_lossy().to_string();
    let error = a.run("file.open", json!({"path": missing})).unwrap_err();
    let msg = a.report_open_failure(&missing, &error);
    assert!(msg.contains("gone.docx") && (msg.to_lowercase().contains("no such file") || msg.contains("os error")), "{msg}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A failed open of a file, with unsaved typing in the window, still names the file and the
/// cause (#1103). (An open no longer waits on "Save changes?": the file opens in its own window.)
#[test]
fn failed_open_with_unsaved_work_names_the_file() {
    let dir = scratch("open-over-unsaved");
    let bad = dir.join("broken.docx").to_string_lossy().to_string();
    std::fs::write(&bad, b"PK\x03\x04 not really a zip").unwrap();
    let mut a = typed();
    a.ui.backstage = true;
    assert!(a.execute_held_back("file.open", json!({"path": bad})).is_err());
    let shown = status_text(&a).unwrap_or_default().to_string();
    assert!(shown.contains("broken.docx"), "{shown}");
    assert!(!a.ui.backstage, "Backstage hides the status bar");
    assert!(body_text(&a).contains(UNSAVED), "the document is untouched");
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #1223: Ctrl+wheel zoomed around the top of the scroll range, not the pointer, so the
/// page slid away under it while zooming; once the pages reflowed into rows the view glided
/// back to the caret. The place under the pointer stays put, rows or not. The events are the
/// ones egui-winit sends on Linux: Ctrl sets both `ctrl` and `command`.
#[test]
fn ctrl_wheel_zoom_keeps_the_place_under_the_pointer() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(4000)})).unwrap();
    a.run("caret.docStart", json!({})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let mut t = 0.0;
    let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    for _ in 0..3 {
        frame(&mut a, Vec::new());
    }
    // The centre of the area the pages show in (inside the rulers), as production's `canvas_rect` was.
    let mut pointer = a.canvas.view_rect().unwrap().center();
    frame(&mut a, vec![egui::Event::PointerMoved(pointer)]);
    let wheel = |y: f32, modifiers| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, y),
        phase: egui::TouchPhase::Move,
        modifiers,
        source: egui::MouseWheelSource::Unknown,
    };
    // Away from the caret, several pages down (a wheel line scrolls less under the framework's page view).
    for _ in 0..240 {
        frame(&mut a, vec![wheel(-1.0, egui::Modifiers::NONE)]);
    }
    for _ in 0..60 {
        frame(&mut a, Vec::new());
    }
    assert!(offset(&a).y > 500.0, "scrolled down: {:?}", offset(&a));
    // The page under `p`.
    let page_under = |a: &WordApp, p: egui::Pos2| a.canvas.page_rects().find(|(_, r)| r.contains(p)).map(|(i, _)| i);
    let mut page = page_under(&a, pointer).unwrap();
    assert!(page >= 3, "a few pages down: page {page}");
    let under = |a: &WordApp, page: usize, pointer: egui::Pos2| (pointer - page_rect(a, page).unwrap().min) / page_scale(a);
    let mut at = under(&a, page, pointer);
    let linux_ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
    frame(&mut a, vec![egui::Event::ModifiersChanged(linux_ctrl)]);
    let (cols, before) = (columns(&page_rects(&a)), page_scale(&a));
    for n in 0..40 {
        let notch = n % 8 == 0;
        if notch {
            // The place each notch keeps is the one under the pointer when it comes. Once the pages reflow into rows
            // they are centred, not kept across (below), so another page can be under the pointer by then, or, with
            // an even number to a row, the gutter at the centre, where there is no place to keep: the pointer moves
            // onto the page right of it first.
            if page_under(&a, pointer).is_none()
                && let Some(right) = a
                    .canvas
                    .page_rects()
                    .map(|(_, r)| r)
                    .filter(|r| r.left() > pointer.x && r.y_range().contains(pointer.y))
                    .map(|r| r.left())
                    .reduce(f32::min)
            {
                pointer.x = right + 20.0;
                frame(&mut a, vec![egui::Event::PointerMoved(pointer)]);
            }
            page = page_under(&a, pointer).unwrap_or_else(|| panic!("frame {n}: the pointer is on a page"));
            at = under(&a, page, pointer);
        }
        frame(&mut a, if notch { vec![wheel(-1.0, linux_ctrl)] } else { Vec::new() });
        let now = under(&a, page, pointer);
        // Across, the place can only be kept while the pages stay one per row: in rows they
        // fit the window side by side and there is nothing to scroll sideways.
        // Within a couple of screen pixels.
        let off = (now - at) * page_scale(&a);
        let across = columns(&page_rects(&a)) > cols || off.x.abs() < 2.0;
        assert!(across && off.y.abs() < 2.0, "frame {n} at {:.0}%: {now:?} on page {page}, was {at:?}", a.canvas.zoom() * 100.0);
    }
    assert!(page_scale(&a) < before * 0.6, "zoomed out: {before} -> {}", page_scale(&a));
    assert!(columns(&page_rects(&a)) > cols, "the pages reflowed into rows on the way");
}

/// The page rectangles on screen, in page order.
fn page_rects(a: &WordApp) -> Vec<egui::Rect> {
    a.canvas.page_rects().map(|(_, r)| r).collect()
}

/// The rectangle of page `page`, when it is on screen.
fn page_rect(a: &WordApp, page: usize) -> Option<egui::Rect> {
    a.canvas.page_rects().find(|(i, _)| *i == page).map(|(_, r)| r)
}

/// The first page on screen.
fn first_page(a: &WordApp) -> Option<egui::Rect> {
    a.canvas.page_rects().next().map(|(_, r)| r)
}

/// Pages per row, read off the rectangles: those level with the first page.
fn columns(rects: &[egui::Rect]) -> usize {
    rects.first().map_or(0, |first| rects.iter().filter(|r| (r.top() - first.top()).abs() < 1.0).count())
}

/// The scroll offset of the view, in screen points.
fn offset(a: &WordApp) -> egui::Vec2 {
    egui::vec2(a.canvas.view.offset.0 as f32, a.canvas.view.offset.1 as f32)
}

/// Screen points per page point.
fn page_scale(a: &WordApp) -> f32 {
    a.canvas.zoom() as f32 * canvas::PX_PER_PT
}

/// Web Layout (#1876): text wraps to the work area, re-wraps once a resize has settled (not on
/// every frame of a drag) and at once on a zoom, and the width is clamped. Print Layout keeps its
/// page width and Draft keeps its 800 pt column after a visit to Web Layout.
#[test]
fn web_layout_fills_the_window_width() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    a.run("text.insert", json!({"text": "A long paragraph that wraps across the work area. ".repeat(200)})).unwrap();
    let mut t = 0.0;
    // One frame `dt` seconds after the last one, in a window `w` wide.
    let mut frame = |a: &mut WordApp, w: f32, dt: f64| {
        t += dt;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 740.0));
        let input = egui::RawInput { events: vec![], time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    let lines = |a: &mut WordApp| {
        a.session
            .layout()
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .map(|it| if let wordcraft_layout::Placed::Lines { l0, l1, .. } = it { l1 - l0 } else { 0 })
            .sum::<usize>()
    };
    let tick = 1.0 / 60.0;
    for _ in 0..3 {
        frame(&mut a, 1400.0, tick);
    }
    let print_lines = lines(&mut a);
    a.run("view.draft", json!({})).unwrap();
    frame(&mut a, 1400.0, tick);
    let draft_lines = lines(&mut a);
    a.run("view.webLayout", json!({})).unwrap();
    frame(&mut a, 1400.0, tick);
    let wide = a.session.view.web_width;
    assert!((900.0..1400.0).contains(&wide), "entering Web Layout fills the window at once, not 800: {wide}");
    let wide_lines = lines(&mut a);
    frame(&mut a, 1400.0, tick);
    assert_eq!(a.session.view.web_width, wide, "no change without a resize");
    let page = first_page(&a).unwrap();
    // The page view keeps its `PageSpacing::side` gutter clear on both sides; the page fills the rest of the window,
    // so it uses the full room up to the PageView gutter rather than the wider rail width, without overflowing.
    let gutter = a.canvas.page_side.expect("page view records its side gutter");
    let rail = craft::ui::runtime::env::Env::of_root(&ctx).tokens.size.rail.width;
    assert!(gutter < rail, "Productivity PageView gutter ({gutter}) is narrower than the rail width ({rail})");
    assert!((page.left() - gutter).abs() <= 4.0, "page starts at the left gutter ({gutter}): {page:?}");
    assert!(page.right() >= 1400.0 - gutter - 4.0 && page.right() <= 1400.0 - gutter + 4.0, "page fills to the right gutter ({gutter}): {page:?}");
    // A window drag: the width holds while the size keeps changing, then follows once it settles.
    for w in [1300.0, 1100.0, 900.0, 700.0, 500.0] {
        frame(&mut a, w, tick);
        assert_eq!(a.session.view.web_width, wide, "no relayout mid-drag at {w}");
    }
    frame(&mut a, 500.0, 0.2);
    let narrow = a.session.view.web_width;
    assert!((144.0..500.0).contains(&narrow), "{narrow}");
    assert!(lines(&mut a) > wide_lines, "narrower window wraps more");
    // A zoom re-wraps at once: at 200% the window holds half as many points.
    frame(&mut a, 1400.0, 0.2);
    frame(&mut a, 1400.0, 0.2);
    let at_100 = a.session.view.web_width;
    a.run("view.zoom", json!({"value": 200})).unwrap();
    frame(&mut a, 1400.0, tick);
    let at_200 = a.session.view.web_width;
    assert!((at_200 - at_100 / 2.0).abs() < 2.0, "200% halves the width: {at_100} -> {at_200}");
    a.run("view.zoom", json!({"value": 100})).unwrap();
    frame(&mut a, 20.0, 0.2);
    frame(&mut a, 20.0, 0.2);
    assert!(a.session.view.web_width >= 144.0);
    frame(&mut a, 1400.0, 0.2);
    frame(&mut a, 1400.0, 0.2);
    a.run("view.draft", json!({})).unwrap();
    frame(&mut a, 1400.0, tick);
    assert_eq!(a.session.layout().pages[0].w, 800.0, "Draft keeps its column after Web Layout");
    assert_eq!(lines(&mut a), draft_lines, "Draft wraps as before Web Layout");
    a.run("view.printLayout", json!({})).unwrap();
    for _ in 0..3 {
        frame(&mut a, 1400.0, tick);
    }
    assert_eq!(lines(&mut a), print_lines, "Print Layout is unchanged");

    // Entering Web Layout before any prior PageView pass still settles to PageView's side gutter.
    let fresh_ctx = crate::test_harness::context();
    let mut fresh = app();
    fresh.run("view.webLayout", json!({})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 740.0));
    for step in 1..=2 {
        let input = egui::RawInput { events: vec![], time: Some(step as f64 * tick), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&fresh_ctx, &mut fresh, input);
    }
    let fresh_gutter = fresh.canvas.page_side.expect("page view records its side gutter on first Web Layout pass");
    let fresh_page = first_page(&fresh).unwrap();
    assert!((fresh_page.left() - fresh_gutter).abs() <= 4.0, "fresh Web Layout starts at the left gutter ({fresh_gutter}): {fresh_page:?}");
    assert!(
        fresh_page.right() >= 1400.0 - fresh_gutter - 4.0 && fresh_page.right() <= 1400.0 - fresh_gutter + 4.0,
        "fresh Web Layout fills to the right gutter ({fresh_gutter}): {fresh_page:?}"
    );
}

/// #688: dragging a selection past the viewport's edge scrolls the view toward the pointer, and the
/// selection keeps growing as the content moves under the stationary pointer.
#[test]
fn dragging_a_selection_past_the_viewport_edge_auto_scrolls() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    // A few pages of text, so there is content below the fold.
    a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
    // The caret starts at the top: the first frames follow the caret, which typing left at the end.
    a.run("caret.docStart", json!({})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
    let mut t = 0.0;
    let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    for _ in 0..10 {
        frame(&mut a, Vec::new());
    }
    let (Some(vp), Some(page)) = (a.canvas.canvas_rect(), first_page(&a)) else { panic!("the canvas laid out") };
    let start = page.left_top() + egui::vec2(120.0, 150.0);
    frame(&mut a, vec![egui::Event::PointerMoved(start)]);
    frame(
        &mut a,
        vec![egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
    );
    assert!(a.canvas.drag_selecting(), "the press started a drag-select");
    let base = a.session.sel.focus.clone();
    // Hold the pointer just past the viewport's bottom edge and let the frames run.
    let below = egui::pos2(vp.center().x, vp.max.y + 40.0);
    for _ in 0..30 {
        frame(&mut a, vec![egui::Event::PointerMoved(below)]);
    }
    assert!(offset(&a).y > 0.0, "the view scrolled toward later content");
    assert!(a.session.sel.focus > base, "the selection followed the content");
    // The pointer never moves; the content keeps scrolling under it, so the selection must
    // keep growing.
    let grown = a.session.sel.focus.clone();
    for _ in 0..15 {
        frame(&mut a, vec![egui::Event::PointerMoved(below)]);
    }
    assert!(a.session.sel.focus > grown, "the selection keeps growing under a stationary pointer");
    frame(
        &mut a,
        vec![egui::Event::PointerButton { pos: below, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }],
    );
    assert!(!a.canvas.drag_selecting());
    assert!(!a.session.sel.is_collapsed(), "the drag left a selection");
}

/// #1013: a press inside the selected comment's balloon (to move the caret in its text) left
/// the balloon, because the canvas under it also "contained" the pointer.
#[test]
fn a_press_inside_the_selected_comment_keeps_it_selected() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    a.run("text.insert", json!({"text": "Some text to comment on. ".repeat(40)})).unwrap();
    a.run("select.word", json!({})).unwrap();
    a.run("review.newComment", json!({"text": "A comment with a few words in it"})).unwrap();
    a.session.view.comments_pane = false;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 700.0));
    let mut t = 0.0;
    let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let (id, card) = *a.canvas.balloon_rects.first().expect("the comment has a balloon");
    frame(&mut a, vec![egui::Event::PointerMoved(card.center())]);
    frame(&mut a, vec![btn(card.center(), true)]);
    frame(&mut a, vec![btn(card.center(), false)]);
    for _ in 0..5 {
        frame(&mut a, Vec::new());
    }
    assert_eq!(a.canvas.selected_balloon(), Some(id), "a click on the balloon selects it");
    let (_, card) = *a.canvas.balloon_rects.iter().find(|(i, _)| *i == id).unwrap();
    let (focus, scroll) = (a.session.sel.clone(), offset(&a));
    // Press, drag a little and release inside the comment's text.
    let (from, to) = (card.left_top() + egui::vec2(30.0, 30.0), card.left_top() + egui::vec2(80.0, 30.0));
    frame(&mut a, vec![egui::Event::PointerMoved(from)]);
    frame(&mut a, vec![btn(from, true)]);
    assert_eq!(a.canvas.selected_balloon(), Some(id), "the press stays in the balloon");
    for _ in 0..3 {
        frame(&mut a, vec![egui::Event::PointerMoved(to)]);
    }
    assert!(!a.canvas.drag_selecting(), "the press didn't start a drag in the document");
    frame(&mut a, vec![btn(to, false)]);
    assert_eq!(a.canvas.selected_balloon(), Some(id), "still selected after the release");
    assert_eq!(a.session.sel, focus, "the document's caret didn't move");
    assert_eq!(offset(&a), scroll);
    // A press on the page itself still leaves the balloon.
    let page = first_page(&a).unwrap();
    let body = page.left_top() + egui::vec2(120.0, 160.0);
    frame(&mut a, vec![egui::Event::PointerMoved(body)]);
    frame(&mut a, vec![btn(body, true)]);
    frame(&mut a, vec![btn(body, false)]);
    assert_eq!(a.canvas.selected_balloon(), None, "a click on the page leaves the balloon");
}

/// #1055: with "only for the selected text" the margin column is gone and just the comment
/// under the caret floats, beside the page in a wide window and over its edge in a narrow one.
#[test]
fn selected_comment_floats_and_keeps_its_clicks() {
    for width in [1700.0_f32, 1000.0] {
        let ctx = crate::test_harness::context();
        let mut a = app();
        a.run("text.insert", json!({"text": "one two three. ".repeat(60)})).unwrap();
        a.run("select.text", json!({"text": "two"})).unwrap();
        a.run("review.newComment", json!({"text": "A comment with a few words in it"})).unwrap();
        a.session.view.comments_pane = false;
        assert!(canvas::markup_width(&a) > 0.0, "the margin is the default, as in Word");
        a.run("review.trackingOptions", json!({"commentDisplay": "selected"})).unwrap();
        assert_eq!(canvas::markup_width(&a), 0.0, "no margin column any more");
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 700.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            crate::test_harness::pass(&ctx, a, input);
        };
        let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        // The caret is in the text after the first comment: not inside its range.
        a.run("caret.docStart", json!({})).unwrap();
        for _ in 0..5 {
            frame(&mut a, Vec::new());
        }
        assert!(a.canvas.balloon_rects.is_empty(), "width {width}: no comment under the caret, no card");
        // Inside the commented word.
        a.run("select.text", json!({"text": "two"})).unwrap();
        a.run("caret.left", json!({})).unwrap();
        for _ in 0..5 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.canvas.balloon_rects.len(), 1, "width {width}: the comment under the caret shows");
        let (id, card) = a.canvas.balloon_rects[0];
        let page = first_page(&a).unwrap();
        assert_eq!(
            card.left() < page.right(),
            width < 1100.0,
            "width {width}: beside the page when the window has room, else over its edge: {card:?} {page:?}"
        );
        if width < 1100.0 {
            let line = a.session.layout().caret_on(&a.session.sel.focus, a.session.page_hint).expect("caret laid out");
            let bottom = page.min.y + (line.top + line.height) * canvas::PX_PER_PT;
            assert!(card.top() >= bottom, "width {width}: over the page the card sits under the commented line: {card:?} vs {bottom}");
        }
        // The #1013 flow on the floating card: click, press inside, drag, release.
        let (caret, scroll) = (a.session.sel.clone(), offset(&a));
        frame(&mut a, vec![egui::Event::PointerMoved(card.center())]);
        frame(&mut a, vec![btn(card.center(), true)]);
        frame(&mut a, vec![btn(card.center(), false)]);
        for _ in 0..5 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.canvas.selected_balloon(), Some(id), "width {width}: a click on the card selects it");
        assert_eq!(a.session.sel.focus.path, caret.focus.path, "width {width}: the document's caret stays in its paragraph");
        let (_, card) = *a.canvas.balloon_rects.iter().find(|(i, _)| *i == id).unwrap();
        let (from, to) = (card.left_top() + egui::vec2(30.0, 30.0), card.left_top() + egui::vec2(80.0, 30.0));
        let caret = a.session.sel.clone();
        frame(&mut a, vec![egui::Event::PointerMoved(from)]);
        frame(&mut a, vec![btn(from, true)]);
        for _ in 0..3 {
            frame(&mut a, vec![egui::Event::PointerMoved(to)]);
        }
        assert!(!a.canvas.drag_selecting(), "width {width}: the press didn't start a drag in the document");
        frame(&mut a, vec![btn(to, false)]);
        assert_eq!(a.canvas.selected_balloon(), Some(id), "width {width}: still selected");
        assert_eq!(a.session.sel, caret, "width {width}: the document's caret didn't move");
        assert_eq!(offset(&a), scroll, "width {width}: nothing scrolled");
        // A click on the page text leaves the card, and the comment goes away with the caret.
        let body = egui::pos2(page.left() + 120.0, page.top() + 40.0 + 150.0);
        frame(&mut a, vec![egui::Event::PointerMoved(body)]);
        frame(&mut a, vec![btn(body, true)]);
        frame(&mut a, vec![btn(body, false)]);
        for _ in 0..5 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.canvas.selected_balloon(), None, "width {width}: a click on the page leaves the card");
    }
}

/// #1019: a double-click in the header area returned before the release ended the drag, so
/// a later press held on the ribbon edge-scrolled the document up.
#[test]
fn a_press_on_the_ribbon_after_a_header_double_click_does_not_scroll() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    a.run("insert.editHeader", json!({})).unwrap();
    a.run("text.insert", json!({"text": "Header"})).unwrap();
    a.run("insert.closeHeader", json!({})).unwrap();
    a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
    let mut t = 0.0;
    let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    a.run("caret.docStart", json!({})).unwrap();
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let (Some(vp), Some(page)) = (a.canvas.canvas_rect(), first_page(&a)) else { panic!("the canvas laid out") };
    let header = egui::pos2(page.left() + 150.0, page.top() + 70.0);
    assert!(vp.contains(header));
    frame(&mut a, vec![egui::Event::PointerMoved(header)]);
    for _ in 0..2 {
        frame(&mut a, vec![btn(header, true)]);
        frame(&mut a, vec![btn(header, false)]);
    }
    frame(&mut a, Vec::new());
    assert!(!a.canvas.drag_selecting(), "the double-click's release ended the drag");
    a.run("insert.closeHeader", json!({})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let ribbon = egui::pos2(vp.center().x, vp.min.y - 40.0);
    let before = offset(&a);
    frame(&mut a, vec![egui::Event::PointerMoved(ribbon), btn(ribbon, true)]);
    for _ in 0..30 {
        frame(&mut a, vec![egui::Event::PointerMoved(ribbon)]);
    }
    assert_eq!(offset(&a), before, "holding a press on the ribbon leaves the document where it is");
    frame(&mut a, vec![btn(ribbon, false)]);
}

/// #3331: Esc out of a header on a later page scrolled back to page 1.
#[test]
fn escape_from_the_header_stays_on_the_page() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
    let mut t = 0.0;
    let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let before = (a.session.sel.clone(), offset(&a));
    assert!(before.1.y > 1000.0, "the caret's last page is in view");
    a.run("insert.editHeader", json!({})).unwrap();
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    frame(&mut a, vec![esc]);
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    assert_eq!(a.session.sel, before.0, "Esc left the header for where the caret was");
    // Entering an empty header scrolls to it (#3201) and Esc scrolls back to the caret, which
    // can land half a pixel from where the view started.
    let drift = offset(&a) - before.1;
    assert!(drift.x.abs() <= 1.0 && drift.y.abs() <= 1.0, "the view stays on that page: {:?} vs {:?}", offset(&a), before.1);
}

/// While editing the header, a single click in the footer edits the footer there (and one in
/// the next page's header goes back to the header) without scrolling. The click used to be
/// hit-tested against the header, so the caret stayed in it and the view scrolled back up.
#[test]
fn a_click_in_the_footer_while_editing_the_header_moves_there() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    a.run("insert.header", json!({"text": "Head"})).unwrap();
    a.run("insert.footer", json!({"text": "Foot"})).unwrap();
    a.run("insert.closeHeader", json!({})).unwrap();
    a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
    let mut t = 0.0;
    let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    a.run("caret.docStart", json!({})).unwrap();
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    a.run("insert.editHeader", json!({"page": 0})).unwrap();
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let page = a.session.layout().pages[0].clone();
    let (header, footer) = (page.header_story.expect("a header"), page.footer_story.expect("a footer"));
    assert_eq!(a.session.sel.focus.story, wordcraft_doc::StoryRef::Part(header));
    let (x, foot_y) = (page.body.x + 20.0, (page.body.bottom() + page.h) / 2.0);
    // Scroll down to page 1's footer a wheel notch at a time: page 1's header goes off screen.
    let vp = a.canvas.canvas_rect().expect("the canvas laid out");
    for _ in 0..100 {
        let at = canvas::page_to_screen(&mut a, 0, x, foot_y).expect("footer laid out");
        if at.y < vp.center().y {
            break;
        }
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        frame(&mut a, vec![egui::Event::PointerMoved(vp.center()), wheel]);
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
    }
    let at = canvas::page_to_screen(&mut a, 0, x, foot_y).expect("footer laid out");
    let head_at = canvas::page_to_screen(&mut a, 0, x, page.body.y / 2.0).expect("header laid out");
    assert!(vp.shrink(100.0).contains(at) && !vp.contains(head_at), "the footer is well on screen, the header isn't: {vp:?} {at:?} {head_at:?}");
    let before = offset(&a);
    frame(&mut a, vec![egui::Event::PointerMoved(at)]);
    frame(&mut a, vec![btn(at, true)]);
    frame(&mut a, vec![btn(at, false)]);
    for _ in 0..40 {
        frame(&mut a, Vec::new());
    }
    assert_eq!(a.session.sel.focus.story, wordcraft_doc::StoryRef::Part(footer), "editing the footer");
    assert_eq!(a.session.page_hint, 0, "on the clicked page");
    assert!((offset(&a).y - before.y).abs() < 1.0, "the view stays put: {before:?} -> {:?}", offset(&a));
    // The next page's header, just below: back in the header, still without scrolling.
    let next = a.session.layout().pages[1].clone();
    let at = canvas::page_to_screen(&mut a, 1, x, next.body.y / 2.0).expect("header laid out");
    assert!(vp.contains(at), "the next page's header is on screen");
    frame(&mut a, vec![egui::Event::PointerMoved(at)]);
    frame(&mut a, vec![btn(at, true)]);
    frame(&mut a, vec![btn(at, false)]);
    for _ in 0..40 {
        frame(&mut a, Vec::new());
    }
    assert_eq!(a.session.sel.focus.story, wordcraft_doc::StoryRef::Part(header), "editing the header again");
    assert_eq!(a.session.page_hint, 1, "on the clicked page");
    assert!((offset(&a).y - before.y).abs() < 1.0, "the view stays put: {before:?} -> {:?}", offset(&a));
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Part(footer)), "Foot");
}

/// #2103: double-clicking the page number in a later page's footer opens the footer there;
/// the view stays on that page instead of jumping to the top of the document.
#[test]
fn double_clicking_a_later_pages_footer_does_not_scroll_to_the_top() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    // A header too, so opening the header instead of the footer can't pass.
    a.run("insert.header", json!({"text": "Head"})).unwrap();
    a.run("insert.closeHeader", json!({})).unwrap();
    a.run("insert.pageNumber", json!({"position": "bottom"})).unwrap();
    for _ in 0..4 {
        a.run("text.insert", json!({"text": "Some body text on this page."})).unwrap();
        a.run("insert.pageBreak", json!({})).unwrap();
    }
    a.run("text.insert", json!({"text": "The last page."})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let mut t = 0.0;
    let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let last = a.session.layout().pages.len() - 1;
    assert!(last >= 3, "several pages");
    let page = a.session.layout().pages[last].clone();
    let (x, y) = (page.body.x + page.body.w / 2.0, (page.body.bottom() + page.h) / 2.0);
    // Scroll the reader to the last page's footer, as with the wheel.
    let at = canvas::page_to_screen(&mut a, last, x, y).expect("footer laid out");
    let vp = a.canvas.canvas_rect().expect("the canvas laid out");
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, vp.center().y - at.y),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    frame(&mut a, vec![egui::Event::PointerMoved(vp.center()), wheel]);
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let at = canvas::page_to_screen(&mut a, last, x, y).expect("footer laid out");
    assert!(vp.contains(at), "the last page's footer is on screen");
    let before = offset(&a);
    assert!(before.y > 1000.0, "scrolled down to the last page: {before:?}");
    frame(&mut a, vec![egui::Event::PointerMoved(at)]);
    // A single click there leaves the caret and the view alone (it used to scroll up to the
    // body text above).
    let caret = a.session.sel.clone();
    frame(&mut a, vec![btn(at, true)]);
    // Already at the press, before the release can decide anything.
    assert_eq!(a.session.sel, caret, "pressing in the footer area doesn't move the caret");
    assert!((offset(&a).y - before.y).abs() < 1.0, "pressing doesn't scroll: {before:?} -> {:?}", offset(&a));
    frame(&mut a, vec![btn(at, false)]);
    for _ in 0..40 {
        frame(&mut a, Vec::new());
    }
    assert_eq!(a.session.sel, caret, "a single click in the footer area doesn't move the caret");
    assert!((offset(&a).y - before.y).abs() < 1.0, "a single click doesn't scroll: {before:?} -> {:?}", offset(&a));
    for _ in 0..2 {
        frame(&mut a, vec![btn(at, true)]);
        frame(&mut a, vec![btn(at, false)]);
    }
    for _ in 0..30 {
        frame(&mut a, Vec::new());
    }
    let footer = page.footer_story.expect("the clicked page has a footer");
    assert!(page.header_story.is_some_and(|h| h != footer), "the page has a separate header: {:?}", page.header_story);
    assert_eq!(a.session.sel.focus.story, wordcraft_doc::StoryRef::Part(footer), "editing the clicked page's footer");
    assert_eq!(a.session.page_hint, last, "on the clicked page");
    assert!((offset(&a).y - before.y).abs() < 1.0, "the view stays put: {before:?} -> {:?}", offset(&a));
}

/// #3734: `ui.resize` refuses sizes that aren't positive and clamps the rest to a window the
/// GPU can draw, replying with the size it asked for.
#[test]
fn control_resize_rejects_bad_sizes_and_clamps_huge_ones() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    let mut resize = |params: Value| {
        let (req, _reply) = ControlRequest::new("ui.resize", params);
        let mut reply = Value::Null;
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let Reply::Done(v) = control::handle(&mut a, ui.ctx(), &req) else { panic!("ui.resize replies at once") };
            reply = v;
        });
        let sent: Vec<egui::Vec2> = out
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .filter_map(|c| if let egui::ViewportCommand::InnerSize(s) = c { Some(*s) } else { None })
            .collect();
        out.drop_without_applying_deltas();
        (reply, sent)
    };
    // 1e300 is past f32: infinite once read, so refused too.
    for bad in [
        json!({"width": -5, "height": 600}),
        json!({"width": 0, "height": 0}),
        json!({"width": 1200, "height": -1}),
        json!({"width": 1e300, "height": 600}),
    ] {
        let (v, sent) = resize(bad.clone());
        assert_eq!(v["ok"], false, "{bad} is refused: {v}");
        assert!(sent.is_empty(), "{bad} sends no size: {sent:?}");
    }
    let max = ctx.input(|i| i.max_texture_side) as f32 / ctx.pixels_per_point();
    let (v, sent) = resize(json!({"width": 1e9, "height": 600}));
    assert_eq!(sent, [egui::vec2(max, 600.0)], "{v}");
    assert_eq!(v["result"], json!({"width": max, "height": 600.0}));
    let (v, sent) = resize(json!({"width": 10, "height": 10}));
    assert_eq!(sent, [window_geometry::MIN_SIZE], "{v}");
    let (v, sent) = resize(json!({"width": 1200, "height": 800}));
    assert_eq!(sent, [egui::vec2(1200.0, 800.0)]);
    assert_eq!(v["result"], json!({"width": 1200.0, "height": 800.0}));
}

/// Issue #123: zoomed out, pages sit side by side, and clicks map to the page under the pointer.
#[test]
fn zoomed_out_pages_sit_side_by_side_and_clicks_land_on_them() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    for _ in 0..3 {
        a.run("insert.pageBreak", json!({})).unwrap();
    }
    a.run("text.insert", json!({"text": "Last page"})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let mut t = 0.0;
    let mut frame = |a: &mut WordApp| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { time: Some(t), screen_rect: Some(screen), ..Default::default() };
        crate::test_harness::pass(&ctx, a, input);
    };
    for _ in 0..3 {
        frame(&mut a);
    }
    // The whole layout, as production's `page_rects` held it: the canvas places only the pages in view.
    let rects = a.canvas.laid_page_rects().to_vec();
    assert_eq!(rects.len(), 4);
    assert!(rects[1].top() > rects[0].bottom(), "100%: one page per row");
    a.run("view.zoom", json!({"value": 30})).unwrap();
    for _ in 0..3 {
        frame(&mut a);
    }
    let rects = a.canvas.laid_page_rects().to_vec();
    assert_eq!(columns(&rects), 4, "30%: all four pages fit across: {rects:?}");
    assert!((rects[3].top() - rects[0].top()).abs() < 1.0 && rects[3].left() > rects[2].right(), "{rects:?}");
    // A click inside the last page's text puts the caret on that page.
    let p = canvas::page_to_screen(&mut a, 3, 100.0, 80.0).unwrap();
    assert!(rects[3].contains(p), "{p:?} in {:?}", rects[3]);
    let pos = canvas::pos_from_screen(&mut a, p).unwrap();
    let caret = a.session.layout().caret_on(&pos, 3).unwrap();
    assert_eq!(caret.page, 3);
}

const UNSAVED: &str = "My unsaved novel chapter";

/// An app whose new document has unsaved typing.
fn typed() -> WordApp {
    typed_scripted().0
}

/// [`typed`], with the file service that answers its picks.
fn typed_scripted() -> (WordApp, Arc<ScriptedFiles>) {
    let files = Arc::new(ScriptedFiles::new());
    let mut a = app_with_files(files.clone());
    a.run("text.insert", json!({"text": UNSAVED})).unwrap();
    assert!(a.session.dirty);
    (a, files)
}

fn body_text(a: &WordApp) -> String {
    a.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

/// The reason the Save Changes prompt shows (a failed save), if that prompt is open.
fn prompt_message(a: &WordApp) -> Option<String> {
    // PLAN(shell-w3): sh.K10 `reason` field of `UnsavedChangesDialog::json_fields`.
    a.closing.as_ref().and_then(|c| c.prompt)?;
    let (_, fields) = open_dialog(a)?;
    Some(fields.get("reason").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// The question the window is asking: Save Changes while a close waits (sh.A7), else the key of an
/// open dialog the tests ask about.
fn prompt(a: &WordApp) -> Option<&'static str> {
    // The close operation is the app's, the question is the drawn window's: a prompt asked in the window of a document
    // that another window replaced is no longer the question this window asks.
    if a.closing.as_ref().and_then(|c| c.prompt).is_some_and(|id| a.dialog_host.get_by_id::<craft::ui::dialog::UnsavedChangesDialog>(id).is_some()) {
        return Some("saveChanges");
    }
    let (key, _) = open_dialog(a)?;
    ["password", "mergeRecords", "printRecords"].into_iter().find(|k| *k == key)
}

/// Cut and Copy reach the system clipboard however they are run — the ribbon, a context menu,
/// the native menu or Cmd+C/X — not only the key event, which used to be the sole writer.
#[test]
fn cut_and_copy_reach_the_system_clipboard() {
    // PLAN(platform-w10): pf.C-P10 `copied` reads the installed `MemoryClipboard`, not egui's `CopyText`.
    let copied = |ctx: &egui::Context| -> Vec<String> {
        ctx.output(|o| {
            o.commands
                .iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::CopyText(t) => Some(t.clone()),
                    _ => None,
                })
                .collect()
        })
    };
    let ctx = crate::test_harness::context();
    let mut a = typed();
    a.ctx = Some(ctx.clone());
    a.run("select.all", json!({})).unwrap();
    let text = a.session.selected_text();
    assert!(!text.is_empty());
    a.run("edit.cut", json!({})).unwrap();
    assert!(body_text(&a).is_empty(), "cut deleted the selection");
    assert!(copied(&ctx).contains(&text), "cut put {text:?} on the clipboard: {:?}", copied(&ctx));

    a.run("edit.undo", json!({})).unwrap();
    a.run("select.all", json!({})).unwrap();
    a.run("edit.copy", json!({})).unwrap();
    assert_eq!(body_text(&a), text, "copy kept the text");
    assert!(copied(&ctx).contains(&text), "copy put {text:?} on the clipboard: {:?}", copied(&ctx));
}

/// #2276: copying a selected picture, which has no text, still replaces what the system
/// clipboard held, so Paste inserts the picture instead of the text copied before it.
#[test]
fn copying_a_picture_replaces_the_clipboards_old_text() {
    // The system clipboard is the installed (memory) clipboard; Copy offers to it through the runtime.
    let last_copied = |ctx: &egui::Context| -> Option<String> {
        craft::ui::runtime::commands::ClipboardHandle::of(ctx).read_text().ok().flatten().filter(|t| !t.is_empty())
    };
    // The default test spec has no readable clipboard: install a memory one.
    use craft::platform::clipboard::{Clipboard, MemoryClipboard};
    let ctx = crate::test_harness::context_with(|| crate::test_harness::install_spec().clipboard(Clipboard::new(MemoryClipboard::default())));
    let mut a = typed();
    a.ctx = Some(ctx.clone());
    a.run("select.all", json!({})).unwrap();
    a.run("edit.copy", json!({})).unwrap();
    assert_eq!(last_copied(&ctx).as_deref(), Some(UNSAVED));

    a.run("select.collapse", json!({"end": true})).unwrap();
    let data = wordcraft_engine::cmd::insert::base64_encode(&png_bytes([200, 30, 30, 255]));
    a.run("insert.picture", json!({"data": data})).unwrap();
    assert!(crate::objects::selected(&a).is_some());
    a.run("edit.copy", json!({})).unwrap();
    assert_eq!(last_copied(&ctx), None, "the old text is gone from the system clipboard");

    // The system clipboard now holds no text, so Paste finds no text to send: our copy.
    a.run("select.collapse", json!({"end": true})).unwrap();
    a.run("edit.paste", json!({})).unwrap();
    assert_eq!(object_count(&mut a), 2, "Paste duplicated the picture");
    assert_eq!(body_text(&a).matches(UNSAVED).count(), 1, "and didn't paste the old text");
}

/// #1265: every user entry point publishes Cut to the one clipboard sink — the ribbon's button
/// and a real Cmd/Ctrl+X key event, not only the generic command dispatch the sink sits behind.
#[test]
fn every_cut_entry_point_publishes_to_the_clipboard() {
    // The sink is the runtime's clipboard (a memory one here), not egui's `CopyText` output.
    use craft::platform::clipboard::{Clipboard, MemoryClipboard};
    let spec = || crate::test_harness::install_spec().clipboard(Clipboard::new(MemoryClipboard::default()));
    let copied = |ctx: &egui::Context| -> Vec<String> {
        craft::ui::runtime::commands::ClipboardHandle::of(ctx).read_text().ok().flatten().into_iter().collect()
    };

    // The ribbon's Cut button runs the command like every ribbon control.
    let ctx = crate::test_harness::context_with(spec);
    let mut a = typed();
    a.ctx = Some(ctx.clone());
    a.run("select.all", json!({})).unwrap();
    let text = a.session.selected_text();
    a.run("edit.cut", json!({})).unwrap();
    assert!(body_text(&a).is_empty(), "ribbon Cut deleted the selection");
    assert!(copied(&ctx).contains(&text), "ribbon Cut published {text:?}: {:?}", copied(&ctx));

    // A real Cut key event through the app's key handling.
    let mut h = crate::test_harness::builder_with(spec).with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| typed());
    for _ in 0..6 {
        h.step();
    }
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    h.state_mut().run("select.all", json!({})).unwrap();
    h.step();
    h.event(egui::Event::Cut);
    h.step();
    h.step();
    let via_key = copied(&h.ctx);
    assert!(via_key.iter().any(|t| t.contains("My unsaved novel")), "key event Cut: {via_key:?} body={:?}", body_text(h.state()));
}

/// A fresh scratch folder for one test.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A .docx on disk that the app didn't write.
fn docx_on_disk(dir: &std::path::Path, text: &str) -> std::path::PathBuf {
    let path = dir.join("contract.docx");
    let mut s = Session::new(wordcraft_doc::Document::from_text(text));
    s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
    path
}

/// Mod+N (and the File › New tiles) open a new document beside unsaved work, asking nothing:
/// nothing is replaced.
#[test]
fn new_opens_beside_unsaved_work() {
    let mut a = typed();
    let first = a.session.document_id();
    a.run("file.new", json!({"template": "letter"})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(!body_text(&a).contains(UNSAVED));
    assert!(!a.session.dirty);
    assert!(!body_text(&a).trim().is_empty(), "the template the user picked was used");
    a.run("view.switchWindows", json!({"id": first})).unwrap();
    assert!(body_text(&a).contains(UNSAVED) && a.session.dirty, "the unsaved document is still open");
}

#[test]
fn a_clean_document_is_replaced_without_asking() {
    let mut a = app();
    a.run("file.new", json!({"template": "letter"})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(!body_text(&a).trim().is_empty());
}

/// #5299: a password-protected file named on the command line, opened after the first blank
/// document was replaced (a region chosen in File › Options), still asks for its password: the
/// window followed the replacement at once, so the first frame didn't take it for a switch of
/// document and drop the Password box.
#[test]
fn a_command_line_file_still_asks_for_its_password_after_the_startup_document_is_replaced() {
    let dir = scratch("password-startup");
    let path = dir.join("locked.docx");
    let mut s = Session::new(wordcraft_doc::Document::from_text("Locked away"));
    s.run("file.save", &json!({"path": path.to_string_lossy(), "password": "sesame"})).unwrap();

    let mut a = app();
    let blank = a.session.blank_document();
    a.replace_startup_document(blank);
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(prompt(&a), Some("password"));
    // The frames that follow keep it, and the password opens the file.
    a.sync_document();
    a.sync_document();
    assert_eq!(prompt(&a), Some("password"), "the box survives the first frames");
    let params = open_dialog(&a).map(|(_, fields)| fields["params"].clone()).expect("the password dialog is open");
    let mut with = params;
    with["password"] = json!("sesame");
    a.run("file.open", with).unwrap();
    assert!(body_text(&a).contains("Locked away"), "{}", body_text(&a));
}

/// A template WordCraft can be handed, saved by the engine with `text` (and `password`).
fn template_file(path: &std::path::Path, text: &str, password: Option<&str>) -> Vec<u8> {
    let mut s = Session::new(wordcraft_doc::Document::from_text(text));
    s.run("file.save", &json!({"path": path.to_string_lossy(), "password": password})).unwrap();
    std::fs::read(path).unwrap()
}

/// #3433: a Word template the platform hands the app (the command line, a double-click in the
/// file manager, the Finder) starts a new, untitled document based on it, in place of the
/// untouched blank document, as Word does; saving can't write over the template. A document is
/// still opened as itself, and File › Open still opens a template itself, to edit it.
#[test]
fn a_template_the_platform_hands_the_app_starts_a_new_document() {
    let dir = scratch("platform-template");
    let path = dir.join("Letterhead.DOTX");
    let bytes = template_file(&path, "Letterhead text", None);
    let name = path.to_string_lossy().to_string();

    let mut a = app();
    let r = a.open_from_platform(&name, None).unwrap();
    assert_eq!(r.get("pending"), None, "{r}");
    assert_eq!(a.session.documents().len(), 1, "the blank startup document gave way");
    assert_eq!(a.session.path, None);
    assert_eq!(a.session.doc.settings.attached_template.as_deref(), Some(name.as_str()));
    assert!(body_text(&a).contains("Letterhead text"));
    assert_eq!(a.title_stem(), "Document1");
    a.run("text.insert", json!({"text": "Dear reader, "})).unwrap();
    let _ = a.run("file.save", json!({}));
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "Save never writes over the template");
    assert_eq!(a.session.path, None);

    // A document opens as itself, beside the new one.
    let doc = dir.join("Report.docx");
    template_file(&doc, "Report text", None);
    a.open_from_platform(&doc.to_string_lossy(), None).unwrap();
    assert_eq!(a.session.path.as_deref(), Some(doc.as_path()));
    // File › Open edits the template itself.
    let mut b = app();
    b.open_file(&name, None).unwrap();
    assert_eq!(b.session.path.as_deref(), Some(path.as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #3433: a protected template from the platform asks for its password, and the password box
/// retries the command that asked, so the template still starts a new document instead of
/// opening itself.
#[test]
fn a_protected_template_from_the_platform_asks_for_its_password_and_stays_a_new_document() {
    let dir = scratch("platform-template-password");
    let path = dir.join("Locked.dotm");
    let bytes = template_file(&path, "Locked template", Some("sesame"));
    let name = path.to_string_lossy().to_string();

    let mut a = app();
    a.open_from_platform(&name, None).unwrap();
    assert_eq!(prompt(&a), Some("password"));
    let (_, fields) = open_dialog(&a).expect("the password dialog is open");
    let (then, params) = (fields["then"].as_str().unwrap_or_default().to_string(), fields["params"].clone());
    assert_eq!(then, "file.newFromTemplate");
    let mut with = params;
    with["password"] = json!("sesame");
    a.execute(&then, with).unwrap();
    assert!(body_text(&a).contains("Locked template"), "{}", body_text(&a));
    assert_eq!((a.session.path.as_ref(), a.session.password.is_some()), (None, false));
    assert_eq!(a.session.documents().len(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Open (after the file is picked), recent files and dropped files all go through `file.open`:
/// the file opens beside unsaved work, asking nothing, and opening it again shows it.
#[test]
fn open_opens_beside_unsaved_work() {
    let dir = scratch("open");
    let path = docx_on_disk(&dir, "Signed contract");
    let mut a = typed();
    let first = a.session.document_id();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(body_text(&a).contains("Signed contract"));
    assert_eq!(a.session.path.as_deref(), Some(path.as_path()));
    let opened = a.session.document_id();
    a.run("view.switchWindows", json!({"id": first})).unwrap();
    assert!(body_text(&a).contains(UNSAVED) && a.session.dirty);
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(a.session.document_id(), opened, "an open file is shown, not opened twice");
    assert_eq!(a.session.documents().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Mod+O without a path only shows the file picker, so it doesn't ask yet.
#[test]
fn open_without_a_path_only_shows_the_picker() {
    let mut a = typed();
    a.run("file.open", json!({})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(body_text(&a).contains(UNSAVED));
}

/// Mod+W ran `file.close`, which quit at once; then it asked first, but still quit the whole
/// app. #1712: as in Word, it closes the document and the window stays, showing no page, until
/// File › New or Open.
#[test]
fn close_asks_then_closes_the_document_and_keeps_the_window() {
    let mut a = typed();
    a.run("file.close", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
    assert!(a.has_document() && body_text(&a).contains(UNSAVED), "Cancel keeps the document");

    a.run("file.close", json!({})).unwrap();
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(!a.quit_requested, "the window stays");
    assert!(!a.has_document() && !a.session.dirty && !a.ui.backstage, "no document, no page");
    assert!(a.close_requested(), "closing the empty window asks nothing");
    a.run("file.close", json!({})).unwrap();
    assert!(!a.quit_requested && !a.has_document(), "nothing left to close");
    a.run("file.new", json!({})).unwrap();
    assert!(a.has_document(), "File › New goes on from there");

    // A clean document closes straight away.
    let mut clean = app();
    clean.run("file.close", json!({})).unwrap();
    assert_eq!(prompt(&clean), None);
    assert!(!clean.quit_requested && !clean.has_document());
}

/// A direct programmatic `file.close` (no prompt, no control channel) never discards unsaved
/// changes either: it is refused unless `force` says so, as over the control channel.
#[test]
fn a_direct_close_keeps_unsaved_changes_unless_forced() {
    let mut a = typed();
    for params in [json!({}), json!({"quit": true}), json!({"force": "yes"})] {
        let e = a.execute("file.close", params.clone()).unwrap_err();
        assert!(e.contains("unsaved") && e.contains("force"), "{params}: {e}");
        assert!(a.has_document() && !a.quit_requested && body_text(&a).contains(UNSAVED), "{params}");
    }
    a.execute("file.close", json!({"force": true})).unwrap();
    assert!(!a.has_document() && !a.quit_requested);
}

/// The window's close button quit at once; now it asks (the host cancels the close on false).
#[test]
fn window_close_asks_first() {
    assert!(app().close_requested(), "a clean document closes");

    let mut a = typed();
    assert!(!a.close_requested());
    assert_eq!(prompt(&a), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
    assert!(!a.close_requested(), "asked again on the next click");

    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(a.quit_requested);
    assert!(a.close_requested(), "the close that follows goes ahead");
    assert!(body_text(&a).contains(UNSAVED), "nothing was replaced on the way out");
}

/// #1445: the window's X on an edited document with a file → Save writes the file and lets
/// the window close.
#[test]
fn window_close_save_writes_the_file_and_quits() {
    let dir = scratch("close-save");
    let path = docx_on_disk(&dir, "Draft");
    let mut a = app();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Revised "})).unwrap();
    assert!(!a.close_requested());
    assert_eq!(prompt(&a), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    let on_disk = wordcraft_engine::io::open_path(&path).unwrap().plain_text(wordcraft_doc::StoryRef::Body);
    assert!(on_disk.contains("Revised Draft"), "the edit is on disk: {on_disk:?}");
    assert!(a.quit_requested, "the app may close");
    assert!(a.close_requested(), "the close that follows goes ahead");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save As suggests a Markdown file's own format, as it does a Word file's; anything else is .docx.
#[test]
fn save_as_suggests_a_markdown_files_own_format() {
    let dir = scratch("save-name");
    for (name, suggested) in [("notes.md", "notes.md"), ("notes.markdown", "notes.markdown"), ("notes.MD", "notes.MD"), ("notes.txt", "notes.docx")] {
        let path = dir.join(name);
        std::fs::write(&path, b"Hello\n").unwrap();
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(a.default_save_name(), suggested, "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// #2376: a text file opened, edited and saved still asked to save on close, and Save in that
/// prompt wrote the file but left the window open; only Don't Save closed it.
#[test]
fn a_text_file_saved_over_itself_closes() {
    let dir = scratch("close-txt");
    let path = dir.join("note.txt");
    std::fs::write(&path, b"").unwrap();

    // Saved with Ctrl+S, then closed: nothing to ask.
    let mut a = app();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    a.run("file.save", json!({})).unwrap();
    assert!(a.close_requested(), "a saved text file closes");
    assert_eq!(prompt(&a), None);

    // Edited again, then closed: Save in the prompt writes the file and lets the window close.
    let mut a = app();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Revised "})).unwrap();
    assert!(!a.close_requested());
    assert_eq!(prompt(&a), Some("saveChanges"));
    let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    // The framework's close guard (CloseScope::Quit) answers a finished quit with `quit`.
    assert_eq!(r["quit"], true, "{r}");
    assert!(std::fs::read_to_string(&path).unwrap().contains("Revised Hello"), "the edit is on disk");
    assert!(a.quit_requested, "the app may close");
    assert!(a.close_requested(), "the close that follows goes ahead");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1445: when Save in the close prompt can't write the file, the prompt's answer is an error
/// that says so plainly, the status bar shows it, and the document stays open, still edited.
#[test]
fn window_close_save_that_fails_says_so_and_keeps_the_document() {
    let dir = scratch("close-save-fails");
    let folder = dir.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let path = docx_on_disk(&folder, "Draft");
    let mut a = app();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Revised "})).unwrap();
    // The document's folder is gone and a file has its name: nothing can be written there.
    std::fs::remove_dir_all(&folder).unwrap();
    std::fs::write(&folder, b"not a folder").unwrap();
    assert!(!a.close_requested());
    a.status = None;
    let err = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap_err();
    assert!(err.contains("Couldn't save contract.docx") && err.contains("still open"), "{err}");
    assert!(status_text(&a) == Some(err.as_str()), "the status bar says why: {:?}", status_text(&a));
    assert!(!a.quit_requested, "nothing closes");
    assert!(a.session.dirty && body_text(&a).contains("Revised Draft"), "nothing was discarded");
    assert_eq!(a.session.path.as_deref(), Some(path.as_path()));
    // The prompt comes back with the reason in it: Save again, Don't Save or Cancel.
    assert_eq!(prompt(&a), Some("saveChanges"), "the prompt is open again");
    assert_eq!(prompt_message(&a).as_deref(), Some(err.as_str()), "the prompt says why");
    assert!(a.run("ui.saveChanges", json!({"answer": "save"})).is_err(), "Save again fails again");
    assert_eq!(prompt(&a), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(!a.close_requested(), "the next close asks again");
    assert_eq!(prompt_message(&a).as_deref(), Some(""), "a fresh prompt, no old reason");
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(a.quit_requested, "Don't Save still closes");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1445: a new document → close → Save → Save As to a place that can't be written: the
/// same plain error, the prompt back with the reason, nothing closed or discarded.
#[test]
fn window_close_save_as_that_fails_says_so_and_keeps_the_document() {
    let dir = scratch("close-save-as-fails");
    let blocker = dir.join("folder");
    std::fs::write(&blocker, b"not a folder").unwrap();
    let picked = blocker.join("novel.docx").to_string_lossy().to_string();
    let (mut a, files) = typed_scripted();
    script_pick(&files, Some(picked.clone()));
    assert!(!a.close_requested());
    a.status = None;
    let err = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap_err();
    assert!(err.contains("Couldn't save") && err.contains("still open"), "{err}");
    assert!(status_text(&a) == Some(err.as_str()), "the status bar says why: {:?}", status_text(&a));
    assert!(!a.quit_requested, "nothing closes");
    assert!(a.session.dirty && body_text(&a).contains(UNSAVED), "nothing was discarded");
    assert_eq!(a.session.path, None);
    assert_eq!(prompt(&a), Some("saveChanges"), "the prompt is open again");
    assert_eq!(prompt_message(&a).as_deref(), Some(err.as_str()), "the prompt says why");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1445: the prompt that comes back after a failed save shows the reason on screen.
#[test]
fn save_prompt_shows_why_the_save_failed() {
    use egui_kittest::kittest::NodeT;
    let dir = scratch("prompt-reason");
    let blocker = dir.join("folder");
    std::fs::write(&blocker, b"not a folder").unwrap();
    let (mut a, files) = typed_scripted();
    script_pick(&files, Some(blocker.join("draft.docx").to_string_lossy().to_string()));
    a.run("file.close", json!({})).unwrap();
    let reason = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap_err();
    assert_eq!(prompt_message(&a).as_deref(), Some(reason.as_str()));
    let mut h = harness(a);
    h.run_steps(4);
    // The prompt shows it, and so does the status line the failed save wrote it to (`prompt_message` above is the prompt's own).
    assert!(h.query_all_by_label(reason.as_str()).next().is_some(), "the reason is in the prompt");
    // The accepting button: the platform's word for Save (the framework asks Yes, No, Cancel off macOS). macOS also
    // keeps a hidden, greyed Save in the title bar, so ask for the one on screen.
    let accept = if cfg!(target_os = "macos") { "Save" } else { "Yes" };
    let offered = h
        .query_all_by_role_and_label(egui::accesskit::Role::Button, accept)
        .any(|n| !n.accesskit_node().is_hidden() && !n.accesskit_node().is_disabled());
    assert!(offered, "{accept} again is offered");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #3172: the ribbon's combo boxes name themselves: a screen reader reads their label (Font,
/// Font Size), and the drop-down arrows say what they drop down.
#[test]
fn the_ribbon_boxes_name_themselves_for_a_screen_reader() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    a.run("file.new", json!({"template": "sample"})).unwrap();
    let mut h = harness(a);
    h.run_steps(6);
    for (role, name) in [(Role::ComboBox, "Font"), (Role::ComboBox, "Font Size"), (Role::Button, "Font Color")] {
        assert!(h.query_by_role_and_label(role, name).is_some(), "nothing on the ribbon is called {name:?} with {role:?}");
    }
}

/// #3172: the Paste button is named "Paste": its tooltip adds the key as the platform writes it,
/// so its name no longer carries macOS's "⌘V", which screen readers read on Windows as "Paste (⌘V)".
#[test]
fn the_paste_button_is_named_without_a_mac_key() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    a.run("file.new", json!({"template": "sample"})).unwrap();
    let mut h = harness(a);
    h.run_steps(6);
    assert!(h.query_by_role_and_label(Role::Button, "Paste").is_some(), "the Paste button is called \"Paste\"");
    assert!(h.query_by_role_and_label(Role::Button, "Paste (⌘V)").is_none(), "no Mac key in its name");
}

/// Make WordCraft the default… (#295) goes through the host's hook and reports the outcome in
/// the status bar; without a hook (web, macOS) it's an error and nothing runs.
#[test]
fn make_default_app_needs_the_hosts_hook_and_reports_the_outcome() {
    let mut a = app();
    assert!(a.make_default_app().is_err(), "no hook: an error");

    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = calls.clone();
    a.services.make_default_app = Some(Box::new(move || {
        seen.set(seen.get() + 1);
        Ok(DefaultApp::MadeDefault)
    }));
    assert_eq!(a.make_default_app().unwrap()["default"], true);
    assert_eq!(calls.get(), 1);
    assert!(status_text(&a).is_some_and(|m| m.contains("now the default")), "{:?}", status_text(&a));

    a.services.make_default_app = Some(Box::new(|| Ok(DefaultApp::SettingsOpened)));
    assert_eq!(a.make_default_app().unwrap()["settingsOpened"], true);
    assert!(status_text(&a).is_some_and(|m| m.contains("Default apps")), "{:?}", status_text(&a));

    a.services.make_default_app = Some(Box::new(|| Err(DefaultAppError::Failed("xdg-mime isn't installed".into()))));
    let e = a.make_default_app().unwrap_err();
    assert!(e.contains("xdg-mime isn't installed"), "{e}");
    assert!(status_text(&a).is_some_and(|m| m.contains("xdg-mime isn't installed")), "the status bar says why");
    assert!(!a.session.dirty, "the document is untouched");
}

/// Changing the system's default apps needs the user's click: `ui.makeDefaultApp` through
/// `run` (what the control channel, MCP and macros use) is refused and never reaches the hook.
#[test]
fn programmatic_calls_cannot_change_the_default_apps() {
    let mut a = app();
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = calls.clone();
    a.services.make_default_app = Some(Box::new(move || {
        seen.set(seen.get() + 1);
        Ok(DefaultApp::MadeDefault)
    }));
    let e = a.run("ui.makeDefaultApp", json!({})).unwrap_err();
    assert!(e.contains("click"), "{e}");
    assert_eq!(calls.get(), 0, "the hook never ran");
}

/// File › Options shows Make WordCraft the default… only when the host can do it (#295), and
/// the button runs the host's hook.
#[test]
fn the_default_app_button_shows_only_with_the_hosts_hook() {
    use egui_kittest::kittest::Queryable;
    const LABEL: &str = "Make WordCraft the default for Word documents…";
    for hook in [false, true] {
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut a = app();
        if hook {
            let seen = calls.clone();
            a.services.make_default_app = Some(Box::new(move || {
                seen.set(seen.get() + 1);
                Ok(DefaultApp::MadeDefault)
            }));
        }
        a.run("ui.backstage", json!({"value": true, "page": "options"})).unwrap();
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
        h.run_steps(4);
        assert!(h.query_by_label("Show the community button in the title bar").is_some(), "hook {hook}: the Options page rendered");
        // The row carries the label too (macOS shows it, with Finder's steps instead of a button): ask for the button.
        let button = h.query_all_by_role_and_label(egui::accesskit::Role::Button, LABEL).next().is_some();
        assert_eq!(button, hook, "hook {hook}: the button");
        if hook {
            // The button itself, not the row that carries its label.
            h.get_by_role_and_label(egui::accesskit::Role::Button, LABEL).click();
            h.run_steps(2);
            assert_eq!(calls.get(), 1, "the button ran the hook");
        }
    }
}

/// A prompt about document A outlived an agent showing B instead, and Don't Save then ran
/// the pending Close against B, throwing away B's edits under a question naming A.
#[test]
fn a_prompt_is_dropped_when_its_document_is_replaced() {
    let dir = scratch("stale");
    let path = docx_on_disk(&dir, "Agent document");
    let ctx = crate::test_harness::context();
    let mut a = typed();
    a.run("file.close", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("saveChanges"));
    for command in [
        json!({"command": "file.open", "params": {"path": path.to_string_lossy()}}),
        json!({"command": "text.insert", "params": {"text": "Edited "}}),
    ] {
        let (req, _reply) = ControlRequest::new("engine.execute", command);
        control::handle(&mut a, &ctx, &req);
    }
    // The control path runs the commands; the window's next frame (`logic` shows the active document) notices the
    // replacement and drops the question.
    crate::test_harness::pass(&ctx, &mut a, egui::RawInput::default());
    assert_eq!(prompt(&a), None, "the question was about a document that is gone");

    // An answer that arrives anyway (a click in that frame, an agent) doesn't touch B.
    assert!(a.run("ui.saveChanges", json!({"answer": "dontSave"})).is_err());
    assert!(body_text(&a).contains("Edited Agent document"));
    assert_eq!(a.session.path.as_deref(), Some(path.as_path()));
    assert!(a.session.dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The web shell's leave-page guard learned about unsaved changes only after `logic`, but
/// typing lands in `ui`, so a tab closed right after the first keystroke left without asking.
#[test]
fn the_host_hears_about_edits_made_while_drawing() {
    let reported = std::rc::Rc::new(std::cell::Cell::new(None));
    let mut a = app();
    let seen = reported.clone();
    a.services.on_dirty = Some(Box::new(move |d| seen.set(Some(d))));
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
    for _ in 0..6 {
        h.step();
    }
    assert!(h.state().canvas.focused(&h.ctx), "the page has keyboard focus");
    assert_eq!(reported.get(), Some(false));
    h.event(egui::Event::Text("x".into()));
    h.step();
    assert!(h.state().session.dirty, "the keystroke reached the document");
    assert_eq!(reported.get(), Some(true), "the host was told before the frame ended");
}

#[test]
fn paragraph_alignment_keeps_the_canvas_focused_for_typing() {
    use wordcraft_doc::Align;

    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| typed());
    for _ in 0..6 {
        h.step();
    }

    for (label, alignment, text) in
        [("Align Left", Align::Left, " left"), ("Center", Align::Center, " center"), ("Align Right", Align::Right, " right")]
    {
        h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
        // The click fires the button's command in that pass; the app runs it before the next pass (`raw_input_hook`).
        h.step();
        h.step();

        assert!(h.state().canvas.focused(&h.ctx), "{label}: the canvas keeps keyboard focus");
        let paragraph = h.state().session.doc.para_at(&h.state().session.sel.focus).expect("the caret remains in its paragraph");
        assert_eq!(h.state().session.doc.styles.resolve_para(&paragraph.props).align, alignment);

        h.event(egui::Event::Text(text.into()));
        h.step();
        assert!(body_text(h.state()).ends_with(text), "{label}: typing continues in the document");
    }

    // Every command button does the same, not only alignment: Bold, then type.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Bold").click();
    h.step();
    assert!(h.state().canvas.focused(&h.ctx), "Bold: the canvas keeps keyboard focus");
    h.event(egui::Event::Text(" bold".into()));
    h.step();
    assert!(body_text(h.state()).ends_with(" bold"), "Bold: typing continues in the document");
}

/// Types `text` after a click on `what` and checks it reached the document (#1093).
fn type_after_click(h: &mut egui_kittest::Harness<'_, WordApp>, what: &str, text: &str) {
    h.step();
    h.step();
    h.event(egui::Event::Text(text.into()));
    h.step();
    assert!(h.state().canvas.focused(&h.ctx), "{what}: the page keeps keyboard focus");
    assert!(body_text(h.state()).ends_with(text), "{what}: typing continues in the document");
}

/// #1093: clicks on ribbon and status-bar chrome that isn't a command button (gallery tiles,
/// tab names, empty ribbon space, View check boxes, the status bar's view buttons and
/// language menu) used to take keyboard focus from the page: the caret vanished and typing
/// went nowhere until the page was clicked again.
#[test]
fn clicks_on_ribbon_and_status_bar_chrome_keep_typing_in_the_document() {
    use egui::accesskit::Role;

    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| typed());
    for _ in 0..6 {
        h.step();
    }
    assert!(h.state().canvas.focused(&h.ctx), "the page starts with keyboard focus");

    // A Styles gallery tile: the framework gallery's tiles are its radio group's buttons.
    h.get_by_role_and_label(Role::RadioButton, "Heading 1").click();
    type_after_click(&mut h, "Heading 1 tile", " gallery");
    let para = h.state().session.doc.para_at(&h.state().session.sel.focus).expect("caret paragraph");
    assert_eq!(para.props.style.as_deref(), Some("Heading1"), "the tile applied its style");

    // A ribbon tab name, then a View tab check box.
    // The ribbon's tab names are tabs of its tab list.
    h.get_by_role_and_label(Role::Tab, "View").click();
    type_after_click(&mut h, "View tab", " tab");
    assert_eq!(h.state().ui.tab, "View");
    let ruler = h.state().session.view.ruler;
    h.get_by_role_and_label(Role::CheckBox, "Ruler").click();
    type_after_click(&mut h, "Ruler check box", " ruler");
    assert_ne!(h.state().session.view.ruler, ruler, "the check box toggled the ruler");

    // Empty space at the right end of the ribbon, under the tab strip.
    let tab = h.get_by_role_and_label(Role::Tab, "View").rect();
    let blank = egui::pos2(1425.0, tab.max.y + 25.0);
    for pressed in [true, false] {
        h.hover_at(blank);
        h.event(egui::Event::PointerButton { pos: blank, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    type_after_click(&mut h, "empty ribbon space", " blank");

    // The status bar's view buttons.
    h.get_by_role_and_label(Role::Button, "Web Layout").click();
    type_after_click(&mut h, "Web Layout", " web");
    assert_eq!(h.state().session.view.mode, wordcraft_layout::ViewMode::Web);
    h.get_by_role_and_label(Role::Button, "Print Layout").click();
    type_after_click(&mut h, "Print Layout", " print");
    assert_eq!(h.state().session.view.mode, wordcraft_layout::ViewMode::Print);

    // The language item opens its menu; a click elsewhere on the status bar cancels it.
    let lang_label = crate::ribbon::caret_language(h.state());
    let lang = h.get_by_label(&lang_label).rect();
    h.get_by_label(&lang_label).click();
    h.step();
    h.step();
    // The framework's menus are its overlay layers, not egui popups.
    let menu_open = |ctx: &egui::Context| craft::ui::runtime::overlay::any_open(ctx, craft::ui::runtime::overlay::OpenFilter::Any);
    assert!(menu_open(&h.ctx), "the language menu opened");
    // Bare status bar: past the language item, and past the missing-fonts item after it when the
    // document's fonts aren't installed (Aptos Display for Heading 1, on most machines without
    // Office), however many there are.
    let right = h.query_by_label_contains(" not installed").map_or(lang.max.x, |n| n.rect().max.x);
    let elsewhere = egui::pos2(right + 40.0, lang.center().y);
    for pressed in [true, false] {
        h.hover_at(elsewhere);
        h.event(egui::Event::PointerButton { pos: elsewhere, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    h.step();
    assert!(!menu_open(&h.ctx), "the click closed the language menu");
    type_after_click(&mut h, "language menu cancelled", " lang");

    // Back on Home, a text field still takes the focus: the font size box.
    h.get_by_role_and_label(Role::Tab, "Home").click();
    type_after_click(&mut h, "Home tab", " home");
    let before = body_text(h.state());
    let size_box =
        h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v.trim().parse::<f64>().is_ok())).expect("the font size box").rect();
    for pressed in [true, false] {
        h.hover_at(size_box.center());
        h.event(egui::Event::PointerButton { pos: size_box.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    h.step();
    h.step();
    assert!(!h.state().canvas.focused(&h.ctx), "the font size box took keyboard focus");
    h.event(egui::Event::Text("4".into()));
    h.step();
    assert!(!h.state().canvas.focused(&h.ctx), "the font size box keeps focus while edited");
    assert_eq!(body_text(h.state()), before, "typing went into the font size box, not the page");
}

/// #1538: inserting WordArt leaves its text selected, so Delete erased only the text and the
/// empty box stayed. Escape now selects the box itself, Delete removes it, and a second Escape
/// is back to a caret in the body.
#[test]
fn escape_selects_a_wordart_box_and_delete_removes_it() {
    let mut a = app();
    let id = a.run("insert.wordArt", json!({})).unwrap()["story"].as_u64().unwrap() as u32;
    assert!(canvas::in_text_box(&a), "the WordArt's text is selected after inserting it");
    let mut h = harness(a);
    h.run_steps(4);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).is_some(), "Escape selects the box");
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert_eq!(h.state().session.doc.text_box_anchor(id), None, "Delete removed the WordArt");
    assert!(h.state_mut().session.undo(), "one undo brings it back");
    assert!(h.state().session.doc.text_box_anchor(id).is_some());
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    let s = &h.state().session;
    assert!(s.sel.is_collapsed() && s.sel.focus.story == wordcraft_doc::StoryRef::Body, "a second Escape leaves a caret in the body");
}

/// Save writes the document first, then carries on with what the user asked for.
#[test]
fn save_then_continue() {
    let dir = scratch("save");
    let path = docx_on_disk(&dir, "Draft");
    let mut a = app();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Revised "})).unwrap();
    a.run("file.close", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(!a.has_document(), "the saved document closed");
    let saved = wordcraft_engine::io::open_path(&path).unwrap();
    assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("Revised Draft"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save As to a format that doesn't keep everything (a .png of page 1, plain text) reports
/// `saved` but leaves the document unsaved; the prompt took that as saved and replaced it.
#[test]
fn a_save_that_drops_content_does_not_continue() {
    for ext in ["png", "txt"] {
        let dir = scratch(&format!("lossy-{ext}"));
        let picked = dir.join(format!("novel.{ext}")).to_string_lossy().to_string();
        let (mut a, files) = typed_scripted();
        script_pick(&files, Some(picked.clone()));
        a.run("file.close", json!({})).unwrap();
        a.status = None;
        let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(r["done"], false, "{ext}: the Close was cancelled");
        assert!(dir.join(format!("novel.{ext}")).exists(), "{ext}: the copy was written");
        assert_eq!(prompt(&a), None);
        assert!(body_text(&a).contains(UNSAVED), "{ext}: the document is kept");
        assert!(a.session.dirty);
        assert!(status_text(&a).is_some(), "{ext}: the status bar says why");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Web: a download in a format that doesn't keep everything marked the document saved, so
/// the prompt and the leave-page guard let it go.
#[test]
fn web_downloads_that_drop_content_leave_the_document_unsaved() {
    for name in ["novel.png", "novel.txt", "novel.md"] {
        let (mut a, _saves) = web_picker_app();
        a.execute("file.saveAs", json!({"path": name})).unwrap();
        assert!(a.session.dirty, "{name}");
    }
    for name in ["novel.docx", "novel.docm", "novel.DOTX", "novel.dotm", "novel.odt", "novel.rtf"] {
        let (mut a, _saves) = web_picker_app();
        a.execute("file.saveAs", json!({"path": name})).unwrap();
        assert!(!a.session.dirty, "{name}");
    }
}

/// Web inside a page that keeps the files: the app with `host_save` recording each file it
/// is handed (name, Save As, size).
fn host_app(fail: bool) -> (WordApp, HostSaves) {
    // A scripted save answers the suggested name before the queue, so a failing page is one that
    // answers from the queue alone.
    let files = Arc::new(if fail { ScriptedFiles::new().in_browser() } else { ScriptedFiles::new().in_browser().saving_suggested() });
    if fail {
        files.queue(Picked::Failed("the page is gone".into()));
    }
    let mut a = app_with_files(files.clone());
    a.run("text.insert", json!({"text": UNSAVED})).unwrap();
    (a, HostSaves(files))
}

/// The saves `files` has written, each with the request that asked for it and the bytes written, oldest first.
fn saved(files: &ScriptedFiles) -> Vec<(PickRequest, Vec<u8>)> {
    files
        .requests()
        .into_iter()
        .filter(|request| request.kind == PickKind::Save)
        .filter_map(|request| {
            let name = std::path::PathBuf::from(request.file_name.clone().unwrap_or_default());
            let path = request.directory.as_ref().map_or_else(|| name.clone(), |dir| dir.join(&name));
            files.file(&path).map(|bytes| (request, bytes))
        })
        .collect()
}

/// The files the host page was handed, as (name, Save As, size).
struct HostSaves(Arc<ScriptedFiles>);

impl HostSaves {
    /// The saves so far.
    fn borrow(&self) -> Vec<(String, bool, usize)> {
        saved(&self.0).into_iter().map(|(request, bytes)| (request.file_name.unwrap_or_default(), request.save_as, bytes.len())).collect()
    }
}

/// Open `name` as the host page sends it (bytes of the typed document in that format).
fn host_open(a: &mut WordApp, name: &str) {
    let bytes = wordcraft_engine::io::save_bytes_with(name, &typed().session.doc, None, &Default::default()).unwrap();
    a.execute("file.open", json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)})).unwrap();
    a.run("text.insert", json!({"text": "Edited "})).unwrap();
    assert!(a.session.dirty);
}

/// Save hands the opened file back under its own name and format; Save As says so and
/// becomes the file the next Save writes.
#[test]
fn host_saves_go_back_to_the_opened_file() {
    let (mut a, got) = host_app(false);
    host_open(&mut a, "report.odt");
    a.execute("file.save", json!({})).unwrap();
    assert!(!a.session.dirty);
    a.run("text.insert", json!({"text": "More "})).unwrap();
    a.execute("file.saveAs", json!({"path": "report copy.docx"})).unwrap();
    assert!(!a.session.dirty);
    // The page answers after the hand-over (S-L159): the answer makes the copy the file the next Save writes.
    a.settle_saves();
    a.run("text.insert", json!({"text": "Again "})).unwrap();
    a.execute("file.save", json!({})).unwrap();
    let got = got.borrow();
    let calls: Vec<(&str, bool)> = got.iter().map(|(n, s, _)| (n.as_str(), *s)).collect();
    assert_eq!(calls, [("report.odt", false), ("report copy.docx", true), ("report copy.docx", false)]);
    assert!(got.iter().all(|(_, _, len)| *len > 0));
}

/// A Markdown file goes back as Markdown, which doesn't keep everything: the document stays
/// unsaved, as on the desktop.
#[test]
fn host_save_of_markdown_keeps_its_format_and_stays_unsaved() {
    let (mut a, got) = host_app(false);
    host_open(&mut a, "notes.md");
    a.execute("file.save", json!({})).unwrap();
    assert_eq!(got.borrow().first().map(|(n, s, _)| (n.as_str(), *s)), Some(("notes.md", false)));
    assert!(a.session.dirty);
    assert_eq!(a.session.path.as_deref(), Some(std::path::Path::new("notes.md")));
}

/// An import-only Word 97–2003 file is saved as a .docx, never back over itself (#2445). The
/// host page never had that name, so it's a new file: Save As.
#[test]
fn host_save_of_a_legacy_word_file_writes_a_docx() {
    let (mut a, got) = host_app(false);
    a.session.path = Some("old.doc".into());
    a.execute("file.save", json!({})).unwrap();
    assert_eq!(got.borrow().first().map(|(n, s, _)| (n.as_str(), *s)), Some(("old.docx", true)));
}

/// Saving a document the host page never sent is Save As, so the page doesn't overwrite a
/// file of the same name; the next Save goes back to it.
#[test]
fn host_save_of_an_untitled_document_is_save_as() {
    let (mut a, got) = host_app(false);
    a.execute("file.save", json!({})).unwrap();
    // The page answers after the hand-over (S-L159): the answer records the file the second Save goes back to.
    a.settle_saves();
    a.run("text.insert", json!({"text": "More "})).unwrap();
    a.execute("file.save", json!({})).unwrap();
    let got = got.borrow();
    let calls: Vec<bool> = got.iter().map(|(_, s, _)| *s).collect();
    assert_eq!(calls, [true, false], "{got:?}");
    assert_eq!(got.first().map(|(n, ..)| n), got.get(1).map(|(n, ..)| n));
}

/// Exports without a name are named after the document, not after its file.
#[test]
fn web_exports_are_named_after_the_document() {
    let (mut a, got) = host_app(false);
    host_open(&mut a, "report.docx");
    a.execute("file.exportPdf", json!({})).unwrap();
    a.execute("file.exportPng", json!({})).unwrap();
    let names: Vec<(String, bool)> = got.borrow().iter().map(|(n, s, _)| (n.clone(), *s)).collect();
    assert_eq!(names, [("report.pdf".to_string(), false), ("report.png".to_string(), false)]);
    assert!(a.session.dirty, "an export is a copy");
}

/// Save As in a format that doesn't keep everything (File › Export to PDF or Markdown) is a
/// copy: the host page isn't told the document is now that file.
#[test]
fn host_save_as_a_copy_format_is_not_save_as() {
    let (mut a, got) = host_app(false);
    host_open(&mut a, "report.docx");
    a.execute("file.saveAs", json!({"path": "report.pdf"})).unwrap();
    a.execute("file.saveAs", json!({"path": "report.md"})).unwrap();
    let calls: Vec<(String, bool)> = got.borrow().iter().map(|(n, s, _)| (n.clone(), *s)).collect();
    assert_eq!(calls, [("report.pdf".to_string(), false), ("report.md".to_string(), false)]);
    assert_eq!(a.session.path.as_deref(), Some(std::path::Path::new("report.docx")));
}

/// The page not getting the file is a failed save: the document keeps its changes.
#[test]
fn a_host_save_that_fails_leaves_the_document_unsaved() {
    let (mut a, _) = host_app(true);
    host_open(&mut a, "report.docx");
    // The page answers after the hand-over (S-L159): its failure puts the changes back.
    a.execute("file.save", json!({})).unwrap();
    a.settle_saves();
    assert!(a.session.dirty);
}

/// Macro-enabled documents and templates keep everything (#172), so once saved in this
/// session AutoSave covers them like a .docx.
#[test]
fn autosave_covers_macro_enabled_documents_and_templates() {
    for ext in ["docm", "dotx", "dotm"] {
        let dir = scratch(&format!("autosave-{ext}"));
        let path = dir.join(format!("novel.{ext}"));
        let mut a = typed();
        a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(a.autosaves(), "{ext}");
        a.run("text.insert", json!({"text": "More "})).unwrap();
        a.autosave_tick(monotonic_ms() + 10_000.0);
        assert!(!a.session.dirty, "{ext}: AutoSave wrote the file");
        let saved = wordcraft_engine::io::open_path(&path).unwrap();
        assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("More "), "{ext}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn renaming_a_saved_file_keeps_autosave_and_replaces_its_recent_entry() {
    let dir = scratch("rename-autosave");
    let path = dir.join("notes.docx");
    let renamed = dir.join("Notes v2.docx");
    let mut a = typed();
    a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(a.autosaves());
    a.run("file.rename", json!({"name": "Notes v2"})).unwrap();
    assert_eq!(a.session.path.as_deref(), Some(renamed.as_path()));
    assert!(a.autosaves(), "AutoSave still covers the renamed file");
    assert_eq!(a.ui.recent.first(), Some(&renamed.to_string_lossy().to_string()));
    assert!(!a.ui.recent.contains(&path.to_string_lossy().to_string()), "{:?}", a.ui.recent);
    a.run("text.insert", json!({"text": "More "})).unwrap();
    a.autosave_tick(monotonic_ms() + 10_000.0);
    assert!(!a.session.dirty, "AutoSave wrote the renamed file");
    let saved = wordcraft_engine::io::open_path(&renamed).unwrap();
    assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("More "));
    assert!(!path.exists());

    // A file opened but not saved in this session doesn't gain AutoSave by being renamed.
    let mut b = app();
    b.run("file.open", json!({"path": renamed.to_string_lossy()})).unwrap();
    assert!(!b.saved_here());
    b.run("file.rename", json!({"name": "Notes v3"})).unwrap();
    assert!(!b.saved_here() && !b.autosaves());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Web: the download callback couldn't report a failure, so Save marked the document saved
/// and the prompt went ahead even when no download started.
#[test]
fn a_failed_download_is_not_a_save() {
    let (mut a, saves) = web_picker_app();
    // The host's files service ends a save later (S-L159): Save hands the file over and the
    // answer, read by `settle_saves`, says how it went.
    a.execute("file.save", json!({})).unwrap();
    saves.finish(saves.last(), Ended::Failed("blocked by the browser".into()));
    a.settle_saves();
    assert!(a.session.dirty, "a failed download leaves the document unsaved");

    a.run("file.close", json!({})).unwrap();
    a.status = None;
    // A failed save is reported, not a quiet Cancel (#1445); the Close doesn't run.
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    saves.finish(saves.last(), Ended::Failed("blocked by the browser".into()));
    a.settle_saves();
    let err = prompt_message(&a).unwrap_or_default();
    assert!(err.contains("still open") && err.contains("blocked by the browser"), "{err}");
    assert!(body_text(&a).contains(UNSAVED));
    assert!(a.session.dirty);
    let msg = status_text(&a).unwrap_or_default().to_string();
    assert!(msg.contains("blocked by the browser"), "the status bar says why: {msg:?}");
}

/// #1315: a web app whose `download` hands the save to the browser's picker, which finishes
/// later. Returns the app (with unsaved typing) and the saves its download starts, plus the
/// last ticket started.
fn web_picker_app() -> (WordApp, WebSaves) {
    let files = Arc::new(ScriptedFiles::held().in_browser());
    let mut a = app_with_files(files.clone());
    a.run("text.insert", json!({"text": UNSAVED})).unwrap();
    assert!(a.session.dirty);
    (a, WebSaves(files))
}

/// How a scripted browser save ended.
#[derive(Clone, Debug)]
enum Ended {
    Saved,
    Cancelled,
    Failed(String),
}

/// The saves the browser's picker holds, which a test ends by hand.
struct WebSaves(Arc<ScriptedFiles>);

impl WebSaves {
    /// The saves still open in the picker.
    fn pending(&self) -> usize {
        self.0.open()
    }

    /// The newest save the picker holds: every ask of the service is held and tickets count the asks.
    fn last(&self) -> PickId {
        PickId(self.0.requests().len() as u64)
    }

    /// Ends save `id`.
    fn finish(&self, id: PickId, ended: Ended) {
        let answered = self.0.answer_to(
            id,
            match ended {
                Ended::Saved => Picked::Paths(vec![std::path::PathBuf::from("novel.docx")]),
                Ended::Cancelled => Picked::Cancelled,
                Ended::Failed(why) => Picked::Failed(why),
            },
        );
        assert!(answered, "save {id:?} was open");
    }
}

/// #1315: Save in the prompt before Close waits for the picker: Close runs only once the file
/// is written and closed.
#[test]
fn web_save_before_close_waits_for_the_picker_to_finish() {
    let (mut a, saves) = web_picker_app();
    a.run("file.close", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("saveChanges"));
    let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    assert_eq!(r["pending"], "save", "{r}");
    assert_eq!(saves.pending(), 1);
    a.settle_saves();
    assert!(body_text(&a).contains(UNSAVED), "nothing closes while the picker is open");
    saves.finish(saves.last(), Ended::Saved);
    a.settle_saves();
    assert!(!body_text(&a).contains(UNSAVED), "the Close ran once the save finished");
    assert!(!a.session.dirty);
    assert_eq!(prompt(&a), None);
}

/// #1315: cancelling the picker, or a write/close that fails, keeps the document: its text,
/// its undo, still unsaved, and the prompt back with the reason. Nothing is replaced.
#[test]
fn web_save_before_close_that_is_cancelled_or_fails_keeps_the_document() {
    for (outcome, reason) in [(Ended::Cancelled, "cancel"), (Ended::Failed("the disk is full".into()), "the disk is full")] {
        let (mut a, saves) = web_picker_app();
        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        a.status = None;
        saves.finish(saves.last(), outcome.clone());
        a.settle_saves();
        assert!(body_text(&a).contains(UNSAVED), "{outcome:?}: nothing was replaced");
        assert!(a.session.dirty, "{outcome:?}: still unsaved");
        assert_eq!(prompt(&a), Some("saveChanges"), "{outcome:?}: the prompt is back");
        let message = prompt_message(&a).unwrap_or_default();
        assert!(message.contains("still open") && message.contains(reason), "{outcome:?}: {message}");
        let status = status_text(&a).unwrap_or_default().to_string();
        assert!(status.contains(reason), "{outcome:?}: the status bar says why: {status:?}");
        // The undo history is still there.
        a.execute("edit.undo", json!({})).unwrap();
        assert!(!body_text(&a).contains(UNSAVED), "{outcome:?}: the typing can be undone");
    }
}

/// #1315 review: the prompt's Save is in the picker (ticket 1), the user edits and saves
/// again (ticket 2, which marks the document saved at once). Ticket 1 ending doesn't let the
/// held-back Close close the document while ticket 2 hasn't written its edits.
#[test]
fn web_overlapping_saves_hold_the_replacement() {
    let (mut a, saves) = web_picker_app();
    a.run("file.close", json!({})).unwrap();
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    let first = saves.last();
    a.run("text.insert", json!({"text": "Later edit "})).unwrap();
    a.execute("file.save", json!({})).unwrap();
    assert!(!a.session.dirty, "the second save marked the document saved");
    saves.finish(first, Ended::Saved);
    a.settle_saves();
    assert!(body_text(&a).contains("Later edit") && body_text(&a).contains(UNSAVED), "nothing was replaced");
    assert_eq!(prompt(&a), Some("saveChanges"), "the prompt asks again");
}

/// #1315 review: edited while the picker was open, then the save ends: those edits aren't
/// in the file, so the prompt asks again instead of running Close.
#[test]
fn web_save_that_ends_after_an_edit_asks_again() {
    let (mut a, saves) = web_picker_app();
    a.run("file.close", json!({})).unwrap();
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    a.run("text.insert", json!({"text": "Later edit "})).unwrap();
    saves.finish(saves.last(), Ended::Saved);
    a.settle_saves();
    assert!(body_text(&a).contains("Later edit"), "nothing was replaced");
    assert!(a.session.dirty);
    assert_eq!(prompt(&a), Some("saveChanges"));
    assert_eq!(prompt_message(&a).as_deref(), Some(""));
}

/// #1315 review: the status bar says Close is waiting for the save; a cancelled picker
/// says it was cancelled, not that saving failed.
#[test]
fn web_save_prompt_says_it_waits_and_that_a_cancel_was_a_cancel() {
    let (mut a, saves) = web_picker_app();
    a.run("file.close", json!({})).unwrap();
    a.status = None;
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    let status = status_text(&a).unwrap_or_default().to_string();
    assert!(status.contains("Waiting for the save"), "{status:?}");
    saves.finish(saves.last(), Ended::Cancelled);
    a.settle_saves();
    let message = prompt_message(&a).unwrap_or_default();
    assert!(message.contains("cancelled") && message.contains("still open"), "{message}");
    assert!(!message.contains("Couldn't save"), "a cancel isn't a failure: {message}");
}

/// #1315 review: a document dropped or picked on the web (`file.open` with its bytes) opens
/// beside one whose save is still in the picker, asking nothing.
#[test]
fn web_open_opens_beside_a_save_in_flight() {
    let dir = scratch("web-open-after-save");
    let bytes = std::fs::read(docx_on_disk(&dir, "Picked document")).unwrap();
    let (mut a, _saves) = web_picker_app();
    let first = a.session.document_id();
    a.execute("file.save", json!({})).unwrap();
    let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
    a.run("file.open", json!({"path": "picked.docx", "data": data})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(body_text(&a).contains("Picked document"), "the picked file opened: {:?}", body_text(&a));
    a.run("view.switchWindows", json!({"id": first})).unwrap();
    assert!(body_text(&a).contains(UNSAVED), "the first document is still open");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1315: while the picker save is in flight the document's save isn't confirmed, so Close
/// asks again instead of closing it.
#[test]
fn web_close_while_a_picker_save_is_in_flight_asks_first() {
    let (mut a, _saves) = web_picker_app();
    a.execute("file.save", json!({})).unwrap();
    a.run("file.close", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("saveChanges"));
    assert!(body_text(&a).contains(UNSAVED), "nothing was replaced");
}

/// #1315: a save that ends after another document is shown neither runs the held-back Close
/// on that document nor marks it unsaved.
#[test]
fn web_picker_save_ending_after_the_document_changed_leaves_the_new_one_alone() {
    for outcome in [Ended::Saved, Ended::Failed("gone".into())] {
        let (mut a, saves) = web_picker_app();
        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        // Another document is shown (an agent's or a script's command) while the picker is open.
        a.execute("file.new", json!({})).unwrap();
        a.execute("text.insert", json!({"text": "Second document"})).unwrap();
        a.session.dirty = false;
        let second = a.session.document_id();
        saves.finish(saves.last(), outcome.clone());
        a.settle_saves();
        if matches!(outcome, Ended::Saved) {
            assert!(body_text(&a).contains("Second document"), "{outcome:?}: the held-back Close didn't run on it");
            assert!(!a.session.dirty, "{outcome:?}: the new document isn't marked unsaved");
            assert_eq!(prompt(&a), None, "{outcome:?}");
        } else {
            // The close operation still waits on the first document, which stays open: the guard shows it and asks
            // again with the reason (`ask_prompt`). The new document is only left alone: it is open, unchanged and
            // not marked unsaved, and no Close ran on it.
            assert!(a.session.is_open(second), "{outcome:?}: the held-back Close didn't run on it");
            let (text, dirty) = a.session.with_document(second, |s| (s.doc.plain_text(wordcraft_doc::StoryRef::Body), s.dirty)).unwrap();
            assert!(text.contains("Second document"), "{outcome:?}: the new document keeps its text");
            assert!(!dirty, "{outcome:?}: the new document isn't marked unsaved");
        }
    }
}

/// A new document has no path: Save goes through Save As, and cancelling that cancels the Close.
#[test]
fn save_as_cancelled_keeps_the_document() {
    let (mut a, files) = typed_scripted();
    script_pick(&files, None);
    a.run("file.close", json!({})).unwrap();
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(body_text(&a).contains(UNSAVED));
    assert!(a.session.dirty);
}

#[test]
fn save_as_picked_saves_then_continues() {
    let dir = scratch("saveas");
    let path = dir.join("novel.docx");
    let picked = path.to_string_lossy().to_string();
    let (mut a, files) = typed_scripted();
    script_pick(&files, Some(picked.clone()));
    a.run("file.close", json!({})).unwrap();
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    assert!(!a.has_document(), "closed once saved");
    let saved = wordcraft_engine::io::open_path(&path).unwrap();
    assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains(UNSAVED));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Scripts and agents (control channel, MCP bridge) are never asked anything.
#[test]
fn programmatic_commands_never_prompt() {
    let ctx = crate::test_harness::context();
    let mut a = typed();
    let (req, _reply) = ControlRequest::new("engine.execute", json!({"command": "file.new"}));
    control::handle(&mut a, &ctx, &req);
    assert_eq!(prompt(&a), None);
    assert!(!body_text(&a).contains(UNSAVED));
}

const MAILINGS: [&str; 4] = ["mailings.envelopes", "mailings.labels", "mailings.finish", "mailings.printDocuments"];

/// What the user gave: Envelopes and Labels without anything open their dialogs (#407).
fn mailing(id: &str) -> Value {
    match id {
        "mailings.envelopes" => json!({"delivery": "Ada"}),
        "mailings.labels" => json!({"text": "Ada"}),
        _ => json!({"records": "all"}),
    }
}

/// #3028: Finish & Merge and Print Documents from the ribbon ask which records first; with
/// records given (scripts, the dialog's OK) they merge.
#[test]
fn finish_and_print_documents_ask_which_records() {
    let mut a = app();
    a.run("mailings.recipients", json!({"csv": "Name\nAda\nAlan"})).unwrap();
    for (id, dialog) in [("mailings.finish", "mergeRecords"), ("mailings.printDocuments", "printRecords")] {
        assert_eq!(a.run(id, json!({})).unwrap(), json!({"pending": dialog}));
        assert_eq!(prompt(&a), Some(dialog));
        let fields = open_dialog(&a).map(|d| d.1).unwrap_or_default();
        // The form's fields are typed: the number fields hold numbers, not their text.
        assert!(fields["from"].as_f64() == Some(1.0) && fields["to"].as_f64() == Some(2.0), "{fields}");
        close_dialogs(&mut a, &crate::test_harness::context());
    }
}

/// Envelopes, Labels and Finish & Merge open their result as a new document beside the
/// unsaved one, like New, asking nothing.
#[test]
fn mailings_open_beside_unsaved_work() {
    for id in MAILINGS {
        let mut a = typed();
        a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        let document = a.session.document_id();
        a.run(id, mailing(id)).unwrap();
        assert_eq!(prompt(&a), None, "{id}");
        assert_ne!(a.session.document_id(), document, "{id}: the result is a new document");
        assert_eq!(a.session.path, None);
        a.run("view.switchWindows", json!({"id": document})).unwrap();
        assert!(body_text(&a).contains(UNSAVED) && a.session.dirty, "{id}: the unsaved document is still open");
    }
    // A `path` that isn't text merges into a new document beside this one, like no path at all.
    let mut a = typed();
    a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
    let document = a.session.document_id();
    a.run("mailings.finish", json!({"path": null, "records": "all"})).unwrap();
    assert_eq!(prompt(&a), None);
    assert_ne!(a.session.document_id(), document);
}

/// A clean document is replaced straight away, like New; a merge written to a file
/// (`mailings.finish` with `path`) doesn't replace anything, so it never asks.
#[test]
fn mailings_on_a_clean_document_or_into_a_file_do_not_ask() {
    let dir = scratch("merge-to-file");
    for id in MAILINGS {
        let mut a = app();
        a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        a.run("file.save", json!({"path": dir.join("saved.docx").to_string_lossy()})).unwrap();
        assert!(!a.session.dirty);
        let document = a.session.document_id();
        a.run(id, mailing(id)).unwrap();
        assert_eq!(prompt(&a), None, "{id}");
        assert_ne!(a.session.document_id(), document, "{id}");
    }
    let out = dir.join("merged.docx");
    let mut a = typed();
    a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
    a.run("mailings.finish", json!({"path": out.to_string_lossy()})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(out.exists());
    assert!(body_text(&a).contains(UNSAVED));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The result is a new, untitled document: Save, AutoSave and the title never point at the
/// file the original came from.
#[test]
fn mailings_results_never_save_over_the_original() {
    for id in MAILINGS {
        let dir = scratch(&format!("mailings-{}", id.len()));
        let path = docx_on_disk(&dir, "Signed contract");
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        a.run("file.save", json!({})).unwrap();
        assert!(a.autosaves());
        a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        a.run("text.insert", json!({"text": "Again "})).unwrap();
        let saved = std::fs::read(&path).unwrap();
        a.run(id, mailing(id)).unwrap();
        assert_eq!(prompt(&a), None);

        assert_eq!(a.session.path, None, "{id}");
        assert!(!a.saved_here() && !a.autosaves(), "{id}: AutoSave doesn't cover the result");
        assert_ne!(a.title_stem(), "contract", "{id}: titled as a new document");
        a.run("text.insert", json!({"text": "Note "})).unwrap();
        a.autosave_tick(monotonic_ms() + 10_000.0);
        a.run("file.save", json!({})).unwrap();
        a.run("edit.undo", json!({})).unwrap();
        a.run("edit.undo", json!({})).unwrap();
        a.run("file.save", json!({})).unwrap();
        a.autosave_tick(monotonic_ms() + 20_000.0);
        assert_eq!(std::fs::read(&path).unwrap(), saved, "{id}: the original file is never overwritten");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Scripts and agents (control channel, MCP bridge) replace the document without a question.
#[test]
fn programmatic_mailings_never_prompt() {
    let ctx = crate::test_harness::context();
    for id in MAILINGS {
        let mut a = typed();
        a.execute("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        let document = a.session.document_id();
        let (req, _reply) = ControlRequest::new("engine.execute", json!({"command": id}));
        control::handle(&mut a, &ctx, &req);
        assert_eq!(prompt(&a), None, "{id}");
        assert_ne!(a.session.document_id(), document, "{id}");
        assert_eq!(a.session.path, None, "{id}");
    }
}

/// Review finding: an agent's Undo while a prompt about the mailing result B was up must not
/// bring document A back under B, where Don't Save would throw A away. The result is its
/// own document: Undo stays in B, and Don't Save closes B and shows A, unsaved work intact.
#[test]
fn undo_after_a_mailing_cannot_redirect_a_pending_prompt() {
    for id in MAILINGS {
        let mut a = typed();
        a.execute("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        let first = a.session.document_id();
        a.execute(id, json!({})).unwrap();
        let b = a.session.document_id();
        assert_ne!(b, first, "{id}");
        a.run("text.insert", json!({"text": "Edited result "})).unwrap();
        a.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"), "{id}");

        a.execute("edit.undo", json!({})).unwrap();
        assert_eq!(a.session.document_id(), b, "{id}");
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(!a.session.is_open(b), "{id}: Don't Save closed the document it asked about");
        assert_eq!(a.session.document_id(), first, "{id}");
        assert!(body_text(&a).contains(UNSAVED) && a.session.dirty, "{id}");
    }
}

/// Review finding: with undo kept across the swap, Undo → Save As → Redo put the mailing result
/// under the saved file, and Save or AutoSave then wrote it there.
#[test]
fn redo_after_a_mailing_cannot_put_the_result_under_a_saved_file() {
    for id in MAILINGS {
        let dir = scratch(&format!("mailings-redo-{}", id.len()));
        let path = docx_on_disk(&dir, "Signed contract");
        let copy = dir.join("copy.docx");
        let on_file = |p: &std::path::Path| wordcraft_engine::io::open_path(p).unwrap().plain_text(wordcraft_doc::StoryRef::Body);
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        a.run("file.save", json!({})).unwrap();
        a.run(id, mailing(id)).unwrap();
        assert_eq!(prompt(&a), None, "{id}: the document was saved");
        let result = body_text(&a);

        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body_text(&a), result, "{id}: history starts afresh, like New");
        a.run("file.saveAs", json!({"path": copy.to_string_lossy()})).unwrap();
        let written = on_file(&copy);
        a.run("edit.redo", json!({})).unwrap();
        assert_eq!(body_text(&a), result, "{id}: nothing to redo");
        a.run("file.save", json!({})).unwrap();
        a.autosave_tick(monotonic_ms() + 10_000.0);
        assert_eq!(on_file(&copy), written, "{id}");
        assert!(on_file(&path).contains("Signed contract"), "{id}: the original is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Surfaces finding 3: AutoSave rewrote any opened .docx ~2.5 s after the first keystroke,
/// deleting whatever the reader doesn't model (content controls, charts, macros…).
#[test]
fn opened_files_are_not_autosaved_until_the_user_saves() {
    let dir = scratch("autosave");
    let path = docx_on_disk(&dir, "Original");
    let before = std::fs::read(&path).unwrap();
    let mut a = app();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Typed "})).unwrap();
    a.autosave_tick(monotonic_ms() + 10_000.0);
    assert!(std::fs::read(&path).unwrap() == before, "an opened file is never rewritten behind the user's back");
    assert!(a.session.dirty);

    // After an explicit save, later changes are saved automatically.
    a.run("file.save", json!({})).unwrap();
    a.run("text.insert", json!({"text": "More "})).unwrap();
    assert!(a.session.dirty);
    a.autosave_tick(monotonic_ms() + 10_000.0);
    assert!(!a.session.dirty);
    let saved = wordcraft_engine::io::open_path(&path).unwrap();
    assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("Typed More Original"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #196: a saved .md file showed AutoSave on, but AutoSave writes only formats that keep
/// everything, so edits stayed unsaved. The switch is now off and blocked with a reason, and
/// scripts turning it on get that reason as an error; a .docx saved here is covered.
#[test]
fn autosave_is_blocked_with_a_reason_for_files_it_cannot_keep() {
    let dir = scratch("autosave-md");
    let md = dir.join("notes.md");
    std::fs::write(&md, "# Notes\n\nSome text\n").unwrap();
    let mut a = app();
    assert_eq!(a.autosave_block(), Some(AutoSaveBlock::Unsaved), "a new document has no file yet");
    a.run("file.open", json!({"path": md.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Typed "})).unwrap();
    a.run("file.save", json!({})).unwrap();
    assert!(a.autosave, "the preference stays on");
    assert_eq!(a.autosave_block(), Some(AutoSaveBlock::Format { name: "notes.md".into() }));
    assert!(!a.autosaves(), "the switch shows off");
    let why = a.autosave_block().map(|b| b.reason()).unwrap_or_default();
    assert!(why.contains("notes.md") && why.contains(".docx"), "{why}");
    let err = a.run("file.autosave", json!({"value": true})).unwrap_err();
    assert_eq!(err, why, "scripts are told why");
    assert!(a.run("file.autosave", json!({"value": false})).is_ok(), "turning it off always works");
    a.autosave = true;

    // Save As .docx (what the greyed switch offers) lifts the block.
    let docx = dir.join("notes.docx");
    a.run("file.saveAs", json!({"path": docx.to_string_lossy()})).unwrap();
    assert_eq!(a.autosave_block(), None);
    assert!(a.autosaves());
    a.run("text.insert", json!({"text": "More "})).unwrap();
    a.autosave_tick(monotonic_ms() + 10_000.0);
    assert!(!a.session.dirty, "AutoSave wrote the .docx");
    assert_eq!(a.run("file.autosave", json!({})).unwrap(), json!({"value": false}), "toggling turns it off");
    assert!(!a.autosaves());
    let _ = std::fs::remove_dir_all(&dir);
}

/// #176: in the browser saving is a download, so AutoSave can't keep a file up to date; the
/// switch is blocked with a reason instead of being on while nothing is saved.
#[test]
fn autosave_is_unavailable_in_the_browser() {
    let mut a = app_with_files(Arc::new(ScriptedFiles::new().in_browser()));
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    a.run("file.save", json!({"path": "draft.docx"})).unwrap();
    assert_eq!(a.autosave_block(), Some(AutoSaveBlock::Browser));
    assert!(!a.autosaves());
    let err = a.run("file.autosave", json!({"value": true})).unwrap_err();
    assert!(err.contains("browser"), "{err}");
}

/// Cancelling the Open picker (Mod+O) left the document as it was but turned AutoSave off.
#[test]
fn cancelling_open_keeps_autosave() {
    let dir = scratch("autosave-open");
    let path = dir.join("novel.docx");
    let (mut a, files) = typed_scripted();
    script_pick(&files, None);
    a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(a.autosaves());
    a.run("file.open", json!({})).unwrap();
    assert!(a.autosaves(), "the picker was cancelled; nothing was replaced");

    a.run("text.insert", json!({"text": " continues"})).unwrap();
    a.autosave_tick(monotonic_ms() + 10_000.0);
    assert!(!a.session.dirty);
    let saved = wordcraft_engine::io::open_path(&path).unwrap();
    assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("chapter continues"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A failing AutoSave used to be silent (`let _ =`) and retried every 2.5 s.
#[test]
fn autosave_failures_are_reported_once() {
    let dir = scratch("autosave-fail");
    let path = dir.join("gone").join("novel.docx");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut a = typed();
    a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
    // The folder disappears (unplugged drive, network share gone).
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    a.run("text.insert", json!({"text": "More "})).unwrap();
    a.status = None;
    a.autosave_tick(monotonic_ms() + 10_000.0);
    let msg = status_text(&a).unwrap_or_default().to_string();
    assert!(msg.contains("AutoSave"), "the failure shows in the status bar: {msg:?}");
    assert!(a.session.dirty);

    // Not retried until the user saves again.
    a.status = None;
    a.autosave_tick(monotonic_ms() + 20_000.0);
    assert!(status_text(&a).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Web Save of an opened .docm downloads a .docm, so its macros stay (Codex review, docm lane).
#[test]
fn web_save_keeps_the_opened_word_format() {
    let mut doc = wordcraft_doc::Document::new();
    doc.passthrough.insert("word/vbaProject.bin".into(), std::sync::Arc::new(vec![0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3]));
    doc.passthrough.insert("word/vbaData.xml".into(), std::sync::Arc::new(b"<wne:vbaSuppData/>".to_vec()));
    // The docx reader's list of parts the project relates to (wordcraft_docx VBA_RELATED).
    let related = "http://schemas.microsoft.com/office/2006/relationships/wordVbaData\tword/vbaData.xml\tapplication/vnd.ms-word.vbaData+xml\n";
    doc.passthrough.insert("wordcraft:vbaProject.related".into(), std::sync::Arc::new(related.as_bytes().to_vec()));
    let files = Arc::new(ScriptedFiles::new().in_browser().saving_suggested());
    let mut a = app_over(Session::new(doc), files.clone());
    let last_save = || saved(&files).pop().map(|(request, bytes)| (request.file_name.unwrap_or_default(), bytes)).unwrap();
    // A browser save goes back under the opened file's own name (S-L159: the host's files service,
    // not the app, says whether the page holds it); a Word 97-2003 file is the one that can't be
    // written back (`host_save_of_a_legacy_word_file_writes_a_docx`).
    for (opened, expect) in
        [("m.docm", "m.docm"), ("t.DOTM", "t.DOTM"), ("t.dotx", "t.dotx"), ("notes.odt", "notes.odt"), ("readme.txt", "readme.txt")]
    {
        a.session.path = Some(opened.into());
        a.run("file.save", json!({})).unwrap();
        let (name, bytes) = last_save();
        assert_eq!(name, expect);
        let back = wordcraft_engine::io::open_bytes(&name, &bytes).unwrap();
        let macros = name.to_ascii_lowercase().ends_with('m');
        for part in ["word/vbaProject.bin", "word/vbaData.xml"] {
            assert_eq!(back.passthrough.contains_key(part), macros, "{opened}: {part}");
        }
    }
    // Save As with an explicit name is a conversion: a .docx can't hold macros.
    a.session.path = Some("m.docm".into());
    a.run("file.saveAs", json!({"path": "m.docx"})).unwrap();
    let (name, bytes) = last_save();
    assert_eq!(name, "m.docx");
    assert!(!wordcraft_engine::io::open_bytes(&name, &bytes).unwrap().passthrough.contains_key("word/vbaProject.bin"));
}

/// Task panes: two open panes on one edge sit side by side, the first outermost; a pane's ×
/// runs its command; the Styles pane's place and float rect survive a restart (#1756).
#[test]
fn task_panes_share_an_edge_close_and_keep_the_styles_place() {
    use egui_kittest::kittest::Queryable;
    // The framework's pane frame is a group named by the pane's title (a tall one: the ribbon has a Styles group
    // too); its title bar is the group's top row, with the action buttons and the options menu on it.
    let bars = |h: &egui_kittest::Harness<'_, WordApp>| {
        let mut bars: Vec<egui::Rect> = ["Comments", "Styles"]
            .into_iter()
            .flat_map(|title| h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Group).label(title)))
            .map(|n| n.rect())
            .filter(|r| r.height() > 300.0)
            .collect();
        bars.sort_by(|a, b| a.left().total_cmp(&b.left()));
        bars
    };
    // A title bar's options menu and × sit on the pane's top row.
    fn click_button(h: &egui_kittest::Harness<'_, WordApp>, bar: egui::Rect, label: &str) {
        let on_row = |r: egui::Rect| bar.contains_rect(r) && r.center().y < bar.top() + 48.0;
        let node = h.get_all_by_label(label).filter(|n| on_row(n.rect())).min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
        node.unwrap_or_else(|| panic!("no {label} beside the title bar")).click();
    }
    let mut a = app();
    a.run("view.stylesPane", json!({"value": true})).unwrap();
    a.run("view.commentsPane", json!({"value": true})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    let [comments, styles] = bars(&h)[..] else { panic!("two pane title bars") };
    assert!(comments.right() <= styles.left() + 1.0, "side by side");
    assert!(comments.left() > 720.0, "both on the right");
    // Default widths 290 and 250: Styles, opened first, is the outer one.
    assert!((comments.width() - styles.width() - 40.0).abs() < 2.0, "Styles is outermost: {comments:?} {styles:?}");

    click_button(&h, styles, "Close");
    h.run_steps(3);
    assert!(!h.state().session.view.styles_pane, "× ran view.stylesPane");
    assert!(h.state().session.view.comments_pane);
    assert_eq!(bars(&h).len(), 1);

    h.state_mut().run("view.stylesPane", json!({"value": true})).unwrap();
    h.run_steps(3);
    let styles = *bars(&h).last().unwrap();
    // The title row's menu action ("More actions") opens the pane's menu, which is the one named "Pane options".
    click_button(&h, styles, "More actions");
    h.run_steps(2);
    h.get_by_label("Float").click();
    h.run_steps(3);
    assert_eq!(h.state().session.view.styles_pane_dock, wordcraft_engine::StylesPaneDock::Floating);
    let rect = h.state().ui.styles_pane_float.expect("the float rect is kept");

    let saved = serde_json::to_vec(&h.state().prefs()).unwrap();
    let mut next = app();
    next.apply_prefs(serde_json::from_slice(&saved).unwrap());
    assert_eq!(next.session.view.styles_pane_dock, wordcraft_engine::StylesPaneDock::Floating);
    assert_eq!(next.ui.styles_pane_float, Some(rect));

    let mut junk = app();
    junk.apply_prefs(serde_json::from_str(r#"{"stylesPaneDock": "top", "stylesPaneFloat": [1, "x", 3, 4], "navPane": true}"#).unwrap());
    assert_eq!(junk.session.view.styles_pane_dock, wordcraft_engine::StylesPaneDock::Right, "junk reads as the default");
    assert_eq!(junk.ui.styles_pane_float, None);
    assert!(junk.session.view.nav_pane, "the rest of the file still loads");
}

/// The same across windows: Check spelling as you type turned off in a second window on a document
/// stays off when the first window's own view of it comes back, in a window on another document,
/// back in the second window, and in the saved settings (#5740).
#[test]
fn check_spelling_off_in_a_second_window_stays_off_in_every_window() {
    let mut a = app();
    let first = a.windows.shown();
    a.new_window();
    let second = a.windows.shown();
    assert_ne!(first, second, "View › New Window opened a window with its own view");
    a.run("file.options", json!({"checkSpelling": false})).unwrap();

    assert!(a.enter_window(first));
    assert!(!a.session.view.proofing, "the first window's own view, put back, follows the setting");
    a.run("file.new", json!({})).unwrap();
    assert!(!a.session.view.proofing, "a new document's window");
    assert!(a.enter_window(second));
    assert!(!a.session.view.proofing, "back in the second window");
    assert!(a.enter_window(first));
    assert!(!a.session.view.proofing);

    let saved = serde_json::to_vec(&a.prefs()).unwrap();
    let mut b = app();
    b.apply_prefs(serde_json::from_slice(&saved).unwrap());
    assert!(!b.session.view.proofing && !b.session.prefs.check_spelling, "saved off, whichever window was drawn");
}

/// #1861: the registered Interface Theme command drives the same setting as Options:
/// searchable in the registry, settable, toggling without a value, and surviving a restart.
#[test]
fn interface_theme_command_sets_toggles_and_persists() {
    use theme::Appearance;
    let mut a = app();
    let spec = a.session.registry.get("view.interfaceTheme").expect("registered and searchable");
    assert_eq!((spec.label, spec.location), ("Interface Theme", "View › Dark Mode"));
    a.run("view.interfaceTheme", json!({"value": "dark"})).unwrap();
    assert_eq!(a.ui.theme, Appearance::Dark);
    a.run("view.interfaceTheme", json!({"value": "system"})).unwrap();
    assert_eq!(a.ui.theme, Appearance::System);
    a.run("view.interfaceTheme", json!({"value": "light"})).unwrap();
    a.run("view.interfaceTheme", json!({})).unwrap();
    assert_eq!(a.ui.theme, Appearance::Dark, "no value toggles light to dark");
    a.run("view.interfaceTheme", json!({})).unwrap();
    assert_eq!(a.ui.theme, Appearance::Light, "and back");
    assert!(a.run("view.interfaceTheme", json!({"value": "sepia"})).is_err());
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    let mut b = app();
    b.apply_prefs(serde_json::from_str(&saved).unwrap());
    assert_eq!(b.ui.theme, Appearance::Light);
}

/// #822: on a display the system reports at 200 % (a 1080p screen under a bare window manager,
/// where winit guesses the scale from the monitor's DPI), "100%" draws at a true 100 %: one
/// screen pixel per point. Auto keeps the display's scale and says what it draws at. Document
/// zoom is a separate setting.
#[test]
fn interface_size_is_absolute_and_auto_keeps_the_system_scale() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    let doc_zoom = a.session.view.zoom;
    a.ctx = Some(ctx.clone());
    let frame = |a: &mut WordApp, native: f32| {
        let mut input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
        // The interface scale is a root value the runtime applies (`RootEnv::scale`): it comes
        // with the frame's input, as eframe's `raw_input_hook` hands it over.
        craft::ui::shell::embed::before_input(&ctx, |out| a.frame_input(out));
        crate::test_harness::pass(&ctx, a, input);
    };
    let ppp = |a: &mut WordApp, native: f32| {
        // The new factor applies from the next pass.
        frame(a, native);
        frame(a, native);
        ctx.pixels_per_point()
    };
    assert_eq!(ppp(&mut a, 2.0), 2.0, "Auto keeps the display's scale");
    assert_eq!(a.run("ui.interfaceSize", json!({})).unwrap(), json!({"auto": true, "percent": 200}));
    a.run("ui.interfaceSize", json!({"percent": 100})).unwrap();
    assert_eq!(ppp(&mut a, 2.0), 1.0, "100% is one screen pixel per point on a 2× display");
    for native in [1.0, 1.25, 1.5, 2.0, 3.0] {
        for pct in [75u16, 100, 150, 200, 300] {
            a.run("ui.interfaceSize", json!({"percent": pct})).unwrap();
            let got = ppp(&mut a, native);
            assert!((got - f32::from(pct) / 100.0).abs() < 1e-4, "{pct}% on a {native}× display drew at {got}");
            assert_eq!(a.run("ui.interfaceSize", json!({})).unwrap()["percent"], pct, "{pct}% on a {native}× display");
        }
    }
    a.run("ui.interfaceSize", json!({"value": "auto"})).unwrap();
    assert_eq!(ppp(&mut a, 1.5), 1.5);
    assert_eq!(a.session.view.zoom, doc_zoom, "document zoom is a separate setting");
}

/// #822: a display scale egui doesn't know or that is nonsense counts as 1×.
#[test]
fn interface_size_survives_an_unusable_display_scale() {
    use theme::InterfaceSize;
    let size = InterfaceSize::from_percent(125.0).unwrap();
    for native in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -2.0] {
        assert_eq!((size.zoom_factor(native), size.drawn_percent(native)), (1.25, 125), "{native}");
        assert_eq!((InterfaceSize::AUTO.zoom_factor(native), InterfaceSize::AUTO.drawn_percent(native)), (1.0, 100), "{native}");
    }
    assert_eq!(InterfaceSize::AUTO.drawn_percent(f32::MAX), u16::MAX);
}

/// #4030: a system that reports a huge scale (an 11-inch 1366x768 panel on X11) must not make Auto
/// draw an interface larger than the screen holds; the controls stay on screen.
#[test]
fn auto_interface_size_is_capped_to_what_fits_the_screen() {
    use theme::{AUTO_MIN_POINTS, InterfaceSize, auto_scale_limit};
    let limit = auto_scale_limit(Some((1366.0, 768.0)));
    assert!((1.0..1.6).contains(&limit), "{limit}");
    // Scales that fit are left as the system reports them.
    for (screen, natives) in [((1920.0, 1080.0), [1.0, 1.25, 1.5]), ((1366.0, 768.0), [1.0, 1.25, 1.5])] {
        let limit = auto_scale_limit(Some(screen));
        for native in natives {
            assert_eq!(f32::from(InterfaceSize::AUTO.drawn_percent_within(native, limit)), native * 100.0, "{screen:?} {native}");
        }
    }
    assert!((auto_scale_limit(Some((1920.0, 1080.0))) - 2.13).abs() < 0.01);
    for native in [2.0, 3.17] {
        let drawn = f32::from(InterfaceSize::AUTO.drawn_percent_within(native, limit)) / 100.0;
        assert!(1366.0 / drawn >= AUTO_MIN_POINTS.0 - 3.0 && 768.0 / drawn >= AUTO_MIN_POINTS.1 - 3.0, "{native}: {drawn}");
    }
    // A scale that already fits, an unknown screen and junk are left alone.
    assert_eq!(InterfaceSize::AUTO.drawn_percent_within(1.25, limit), 125);
    assert_eq!(auto_scale_limit(None), f32::INFINITY);
    assert_eq!(auto_scale_limit(Some((f32::NAN, 768.0))), f32::INFINITY);
    assert_eq!(InterfaceSize::AUTO.drawn_percent_within(3.0, auto_scale_limit(None)), 300);
    // A screen below the minimum is never scaled under 100 %; a 4K screen at 2x is untouched.
    assert_eq!(auto_scale_limit(Some((640.0, 480.0))), 1.0);
    assert_eq!(InterfaceSize::AUTO.drawn_percent_within(2.0, auto_scale_limit(Some((3840.0, 2160.0)))), 200);
    // A chosen size is the user's and is not capped.
    assert_eq!(InterfaceSize::from_percent(300.0).unwrap().drawn_percent_within(3.0, limit), 300);
}

/// #4030: the zoom is shared, so the smallest screen of all the open windows caps Auto, and a
/// screen's size in pixels gives the same cap whatever scale its window runs at.
#[test]
fn auto_cap_follows_the_smallest_window_screen() {
    use theme::auto_scale_limit_over;
    let small = auto_scale_limit_over([([1366, 768], 1.0)], 1.0);
    let large = auto_scale_limit_over([([3840, 2160], 1.0)], 1.0);
    assert!(small < large);
    assert_eq!(auto_scale_limit_over([([3840, 2160], 1.0), ([1366, 768], 1.0)], 1.0), small);
    // A 1366x768 panel at native scale 3: the pixels, and so the cap, are the same.
    let a = auto_scale_limit_over([([1366, 768], 3.0)], 3.0);
    assert!((a - small).abs() < 1e-4, "{a} {small}");
    assert_eq!(auto_scale_limit_over([], 1.0), f32::INFINITY);
}

/// #4030: with a 1x window on a large screen and a 3x window on a 1366x768 panel, the shared zoom
/// is the one the 3x window needs, so both windows draw within their own screen.
#[test]
fn auto_cap_keeps_every_window_within_its_own_screen_when_scales_differ() {
    use theme::{AUTO_MIN_POINTS, InterfaceSize, auto_scale_limit, usable_scale};
    let windows = [((3840.0, 2160.0), 1.0_f32), ((1366.0 / 3.0, 768.0 / 3.0), 3.0)];
    let monitors = windows.map(|((w, h), own)| ([(w * own).round() as u32, (h * own).round() as u32], own));
    for root in [1.0_f32, 3.0] {
        let limit = theme::auto_scale_limit_over(monitors, root);
        let zoom = InterfaceSize::AUTO.zoom_factor_within(root, limit);
        for ((w, h), own) in windows {
            let pixels_per_point = usable_scale(own) * zoom;
            let (px_w, px_h) = (w * own, h * own);
            assert!(pixels_per_point <= auto_scale_limit(Some((px_w, px_h))) + 1e-4, "root {root}, window {own}: {pixels_per_point}");
            assert!(
                px_w / pixels_per_point >= AUTO_MIN_POINTS.0 - 1.0 && px_h / pixels_per_point >= AUTO_MIN_POINTS.1 - 1.0,
                "root {root}, window {own}"
            );
        }
    }
}

/// #4030 review: where Shift+0 types `=` (German, Swiss and Nordic layouts) egui-winit reports `=`
/// as the key and the zero only as the physical key. That press is Ctrl+Shift+=, Superscript, as
/// on main; it doesn't reset the interface size.
#[cfg(not(target_os = "macos"))]
#[test]
fn ctrl_shift_equals_on_the_zero_key_stays_superscript() {
    let mut a = app();
    a.run("document.setText", json!({"text": "x"})).unwrap();
    a.run("select.all", json!({})).unwrap();
    a.run("ui.interfaceSize", json!({"percent": 300})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
    for _ in 0..6 {
        h.step();
    }
    let chord = egui::Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };
    h.event(egui::Event::Key { key: egui::Key::Equals, physical_key: Some(egui::Key::Num0), pressed: true, repeat: false, modifiers: chord });
    h.step();
    // The chord fires its command in that pass; the app runs it before the next pass (`raw_input_hook`).
    h.step();
    assert_eq!(h.state().ui.interface_size.percent(), Some(300), "Ctrl+Shift+= doesn't reset");
    let pos = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
    let props = h.state().session.doc.para_at(&pos).map(|p| p.props_of_char(0).clone()).unwrap_or_default();
    assert_eq!(props.vert_align, Some(wordcraft_doc::props::VertAlign::Superscript), "Ctrl+Shift+= runs Superscript");
}

/// #980: F11 and View › Full Screen (`view.fullScreen`) take the window to full screen; a
/// second F11 or Escape brings it back.
#[test]
fn f11_and_escape_toggle_full_screen() {
    use egui::ViewportCommand::Fullscreen;
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app());
    for _ in 0..6 {
        h.step();
    }
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    // Run frames and return the full-screen requests the window got.
    let sent = |h: &mut egui_kittest::Harness<WordApp>| {
        let mut out = Vec::new();
        for _ in 0..2 {
            h.step();
            out.extend(h.output().viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, Fullscreen(_))).cloned());
        }
        out
    };
    assert_eq!(sent(&mut h), vec![], "a normal window to start with");
    h.event(key(egui::Key::F11));
    assert_eq!(sent(&mut h), vec![Fullscreen(true)], "F11");
    assert!(h.state().session.view.full_screen);
    h.event(key(egui::Key::F11));
    assert_eq!(sent(&mut h), vec![Fullscreen(false)], "F11 again");
    assert!(!h.state().session.view.full_screen);
    // The command (ribbon, command search, agents), then Escape.
    assert_eq!(h.state_mut().run("view.fullScreen", json!({})).unwrap(), json!({"value": true}));
    assert_eq!(sent(&mut h), vec![Fullscreen(true)], "the command");
    h.event(key(egui::Key::Escape));
    assert_eq!(sent(&mut h), vec![Fullscreen(false)], "Escape");
    // Escape in a normal window doesn't touch the window.
    h.event(key(egui::Key::Escape));
    assert_eq!(sent(&mut h), vec![]);
    // Escape closing a dialog leaves full screen alone.
    h.state_mut().run("view.fullScreen", json!({"value": true})).unwrap();
    h.state_mut().run("ui.dialog", json!({"name": "zoom"})).unwrap();
    assert_eq!(sent(&mut h), vec![Fullscreen(true)]);
    h.event(key(egui::Key::Escape));
    assert_eq!(sent(&mut h), vec![]);
    assert!(open_dialog(h.state()).is_none(), "Escape closed the dialog");
    assert!(h.state().session.view.full_screen, "and only that");
}

/// The Full Screen button tells assistive technology what the command can do: it is enabled
/// exactly when the command is, so the browser build (which keeps full screen to itself)
/// does not announce a control that cannot be activated.
#[test]
fn the_full_screen_button_reports_the_commands_own_enabled_state() {
    use egui_kittest::kittest::{NodeT, Queryable};
    use wordcraft_engine::{CommandSpec, Registry};

    // The browser build's reason for refusing full screen, forced here so the test says
    /// something on every platform.
    fn unavailable(_: &wordcraft_engine::Session) -> Option<&'static str> {
        Some("Use the browser's full screen (F11)")
    }

    let mut a = app();
    let specs: Vec<CommandSpec> = a
        .session
        .registry
        .all()
        .iter()
        .map(|s| {
            if s.id == "view.fullScreen" {
                CommandSpec {
                    id: s.id,
                    label: s.label,
                    location: s.location,
                    shortcut: s.shortcut,
                    params: s.params,
                    mutates: s.mutates,
                    enabled: unavailable,
                    run: s.run,
                }
            } else {
                s.clone()
            }
        })
        .collect();
    a.session.registry = std::sync::Arc::new(Registry::new(specs));

    let mut h = crate::test_harness::builder().with_size(egui::vec2(1920.0, 900.0)).build_eframe(|_| a);
    for _ in 0..6 {
        h.step();
    }
    h.state_mut().run("ui.tab", json!({ "tab": "View" })).unwrap();
    for _ in 0..3 {
        h.step();
    }
    let node = h
        .query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button))
        .into_iter()
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.replace('\n', " ").contains("Full Screen")))
        .expect("the Full Screen button is on the View tab");
    assert!(!crate::app::command_board::command_enabled(h.state(), "view.fullScreen"), "the command is unavailable, as the test arranged it",);
    assert!(node.accesskit_node().is_disabled(), "a control the command refuses is announced as disabled",);
}

/// #980: in full screen, Escape with text selected collapses the selection (as in a normal
/// window) and leaves full screen alone; the next Escape, with nothing selected, leaves it.
#[test]
fn escape_collapses_a_selection_before_leaving_full_screen() {
    use egui::ViewportCommand::Fullscreen;
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app());
    for _ in 0..6 {
        h.step();
    }
    let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let sent = |h: &mut egui_kittest::Harness<WordApp>| {
        let mut out = Vec::new();
        for _ in 0..2 {
            h.step();
            out.extend(h.output().viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, Fullscreen(_))).cloned());
        }
        out
    };
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    h.state_mut().run("text.insert", json!({"text": "Hello world"})).unwrap();
    h.state_mut().run("select.all", json!({})).unwrap();
    h.state_mut().run("view.fullScreen", json!({"value": true})).unwrap();
    assert_eq!(sent(&mut h), vec![Fullscreen(true)]);
    assert!(!h.state().session.sel.is_collapsed());
    h.event(esc.clone());
    assert_eq!(sent(&mut h), vec![], "Escape with a selection stays in full screen");
    assert!(h.state().session.sel.is_collapsed(), "and collapses the selection");
    assert!(h.state().session.view.full_screen);
    h.event(esc);
    assert_eq!(sent(&mut h), vec![Fullscreen(false)], "Escape with nothing selected leaves");
    assert!(!h.state().session.view.full_screen);
}

/// #980: the window leaving full screen by other means (the window manager, the macOS title
/// bar button) turns View › Full Screen off, and entering it turns it on.
#[test]
fn full_screen_follows_the_window() {
    let mut a = app();
    // PLAN(shell-w13): sh.A13 `follow_screen_mode`; the window actions are `ScreenModeFollow`'s.
    a.follow_screen_mode(ScreenMode::FullScreenWithMenuBar);
    assert!(a.session.view.full_screen, "the window went full screen");
    a.follow_screen_mode(ScreenMode::Standard);
    assert!(!a.session.view.full_screen, "and came back");
    // Focus, then the system takes the window out of full screen: Focus ends with it.
    a.run("view.focus", json!({})).unwrap();
    assert!(a.session.view.focus_mode);
    a.follow_screen_mode(ScreenMode::Standard);
    assert!(!a.session.view.focus_mode, "Focus ended with the window's full screen");
    assert!(!a.session.view.full_screen);
}

/// #1678: Focus and Full Screen View put the window in full screen, asked of the system once
/// per change (switching between the two keeps it), and the usual window comes back after.
#[test]
fn focus_and_full_screen_view_ask_for_full_screen_once_per_change() {
    use egui::ViewportCommand::Fullscreen;
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app());
    for _ in 0..6 {
        h.step();
    }
    let sent = |h: &mut egui_kittest::Harness<WordApp>| {
        let mut out = Vec::new();
        for _ in 0..2 {
            h.step();
            out.extend(h.output().viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, Fullscreen(_))).cloned());
        }
        out
    };
    assert_eq!(sent(&mut h), vec![], "a normal window to start with");
    h.state_mut().run("view.focus", json!({})).unwrap();
    assert_eq!(sent(&mut h), vec![Fullscreen(true)]);
    assert_eq!(sent(&mut h), vec![]);
    h.state_mut().run("view.fullScreenView", json!({})).unwrap();
    assert_eq!(sent(&mut h), vec![], "Focus to Full Screen View stays full screen");
    h.state_mut().run("view.fullScreenView", json!({})).unwrap();
    assert_eq!(sent(&mut h), vec![Fullscreen(false)]);
}

/// #312: Dark page darkens only the pages; the interface keeps following its own setting.
#[test]
fn dark_page_leaves_the_interface_theme_alone() {
    // The `System` rows are the runtime's (`ModeChoice::System`); the resolved mode reads `ResolvedRoot.mode`.
    for (setting, dark) in [("light", false), ("dark", true)] {
        let ctx = crate::test_harness::context();
        let mut a = app();
        a.run("ui.theme", json!({"value": setting})).unwrap();
        a.run("view.darkMode", json!({"value": true})).unwrap();
        assert!(a.session.view.dark_mode, "{setting}");
        let mode = a.root_env().resolved(&ctx).mode;
        assert_eq!(mode == craft::theme::Mode::Dark, dark, "{setting}");
        assert_eq!(a.run("ui.theme", json!({})).unwrap()["dark"], dark, "{setting}");
    }
}

fn png_bytes(c: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(20, 10, |_, _| image::Rgba(c));

    wordcraft_geom::picture::encode_format(&image::DynamicImage::ImageRgba8(img).clone(), image::ImageFormat::Png).unwrap()
}

fn insert_picture(a: &mut WordApp, c: [u8; 4]) {
    let data = wordcraft_engine::cmd::insert::base64_encode(&png_bytes(c));
    a.session.run("insert.picture", &json!({"data": data})).unwrap();
}

fn object_count(a: &mut WordApp) -> usize {
    a.session.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(|x| x.len()).unwrap_or(0)
}

/// #965: a picture or icon the person inserts is selected, with handles, as in Word; one a
/// script inserts (control channel, MCP bridge) leaves the caret after it, and typing keeps it.
#[test]
fn inserted_picture_is_selected_for_the_user_not_for_scripts() {
    let data = wordcraft_engine::cmd::insert::base64_encode(&png_bytes([200, 30, 30, 255]));
    for (id, params) in [("insert.picture", json!({"data": data})), ("insert.icon", json!({"id": "sun"}))] {
        let mut a = app();
        a.run(id, params.clone()).unwrap();
        assert!(crate::objects::selected(&a).is_some(), "{id}: the person's picture is selected");
        let mut a = app();
        a.execute(id, params).unwrap();
        assert!(a.session.sel.is_collapsed() && crate::objects::selected(&a).is_none(), "{id}: a script gets the caret after it");
        a.execute("text.newParagraph", json!({})).unwrap();
        a.execute("text.insert", json!({"text": "cap"})).unwrap();
        assert_eq!(object_count(&mut a), 1, "{id}");
    }
}

/// A picture pasted in the browser lands only when the page is the edit target (#1349);
/// dropped or opened files always land.
#[test]
fn pasted_picture_inserts_only_when_the_page_is_the_edit_target() {
    let mut h = paste_harness(app());
    // PLAN(shell-w14): sh.A18 `paste_arrived` runs in the frame; the picture arrives through the installed clipboard.
    let paste = |h: &mut egui_kittest::Harness<'static, WordApp>, c| {
        // Collapse after the last picture so an insertion adds one instead of replacing it.
        h.state_mut().session.run("select.collapse", &json!({"end": true})).unwrap();
        offer_paste(&h.ctx, png_bytes(c));
        h.run_steps(3);
    };
    // The page has focus: inserted once.
    paste(&mut h, [200, 30, 30, 255]);
    assert_eq!(object_count(h.state_mut()), 1);
    h.run_steps(3);
    assert_eq!(object_count(h.state_mut()), 1, "once");
    // A dialog is open.
    h.state_mut().run("ui.dialog", json!({"name": "find"})).unwrap();
    h.run_steps(2);
    assert!(open_dialog(h.state()).is_some());
    paste(&mut h, [1, 2, 3, 255]);
    assert_eq!(object_count(h.state_mut()), 1, "a dialog has the paste");
    let ctx = h.ctx.clone();
    close_dialogs(h.state_mut(), &ctx);
    h.state_mut().canvas.focus_page(&ctx);
    h.run_steps(2);
    // The Backstage is open.
    h.state_mut().ui.backstage = true;
    paste(&mut h, [1, 2, 3, 255]);
    assert_eq!(object_count(h.state_mut()), 1, "the Backstage has the paste");
    h.state_mut().ui.backstage = false;
    h.run_steps(2);
    // A dropped file still inserts.
    h.state_mut().session.run("select.collapse", &json!({"end": true})).unwrap();
    drop_files(h.state_mut(), vec![dropped_bytes("drop.png", png_bytes([5, 5, 5, 255]))]);
    h.run_steps(3);
    assert_eq!(object_count(h.state_mut()), 2);
}

fn paste_harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    use craft::platform::clipboard::Clipboard;
    let spec = || crate::test_harness::install_spec().clipboard(Clipboard::new(LatePaste::default()));
    let mut h = crate::test_harness::builder_with(spec).with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
    for _ in 0..6 {
        h.step();
    }
    h
}

/// Through the real frame loop (#1349): a paste onto the focused page inserts once; the
/// focused page must not look like a text field to the guard.
#[test]
fn a_paste_onto_the_focused_page_inserts_one_picture() {
    let mut h = paste_harness(app());
    assert!(h.state().canvas.focused(&h.ctx));
    offer_paste(&h.ctx, png_bytes([9, 9, 9, 255]));
    h.step();
    assert_eq!(object_count(h.state_mut()), 1);
    h.step();
    assert_eq!(object_count(h.state_mut()), 1, "once");
}

/// A pending Change Picture replaces the selected image instead of inserting (#147).
#[test]
fn change_picture_replaces_instead_of_inserting() {
    let mut a = app();
    insert_picture(&mut a, [200, 30, 30, 255]);
    assert_eq!(object_count(&mut a), 1);
    let before = a.selected_picture_media().unwrap();
    a.change_picture_target = a.selected_picture_media();
    drop_files(&mut a, vec![dropped_bytes("new.png", png_bytes([30, 200, 30, 255]))]);
    a.drain_inbox();
    assert_eq!(object_count(&mut a), 1);
    assert_ne!(a.selected_picture_media().unwrap(), before);
    assert!(a.change_picture_target.is_none());
}

/// Several images dropped together all stay in the document (#1592).
#[test]
fn several_dropped_images_are_all_inserted() {
    let mut a = app();
    drop_files(&mut a, vec![dropped_bytes("a.png", png_bytes([200, 30, 30, 255])), dropped_bytes("b.png", png_bytes([30, 200, 30, 255]))]);
    a.drain_inbox();
    assert_eq!(object_count(&mut a), 2);
}

/// With a Drawing Canvas selected, every image of the drop goes into it (#1592).
#[test]
fn several_dropped_images_all_join_a_selected_canvas() {
    let mut a = app();
    a.session.run("insert.canvas", &json!({})).unwrap();
    drop_files(&mut a, vec![dropped_bytes("a.png", png_bytes([200, 30, 30, 255])), dropped_bytes("b.png", png_bytes([30, 200, 30, 255]))]);
    a.drain_inbox();
    let members = a.session.run("canvas.member", &json!({})).unwrap();
    assert_eq!(members["members"].as_array().map(|m| m.len()), Some(2));
}

/// Without a pending change, inbox images insert; stale targets clear on selection change.
#[test]
fn insert_without_pending_adds_and_stale_target_clears() {
    let mut a = app();
    insert_picture(&mut a, [200, 30, 30, 255]);
    a.session.run("select.collapse", &json!({"end": true})).unwrap();
    drop_files(&mut a, vec![dropped_bytes("second.png", png_bytes([30, 30, 200, 255]))]);
    a.drain_inbox();
    assert_eq!(object_count(&mut a), 2);
    // Stale target (another picture's key) clears instead of replacing the new selection.
    a.change_picture_target = Some("m0000-gone.png".into());
    a.clear_stale_change_picture();
    assert!(a.change_picture_target.is_none());
}

/// `ui.changePicture` needs a selected picture; cancelling the picker clears the target.
#[test]
fn change_picture_guards_and_cancel_clears() {
    let mut a = app();
    assert!(a.run("ui.changePicture", json!({})).is_err());
    assert!(a.change_picture_target.is_none());
    insert_picture(&mut a, [200, 30, 30, 255]);
    // No pickers in tests, so the picker "cancels" and the target clears.
    assert!(a.run("ui.changePicture", json!({})).is_ok());
    assert!(a.change_picture_target.is_none());
}

/// Any unrelated action cancels a pending Change Picture (#147 web-picker cancel case).
#[test]
fn unrelated_command_clears_change_picture_target() {
    let mut a = app();
    insert_picture(&mut a, [200, 30, 30, 255]);
    a.change_picture_target = a.selected_picture_media();
    assert!(a.change_picture_target.is_some());
    let _ = a.run("format.bold", json!({}));
    assert!(a.change_picture_target.is_none());
}

/// Add Font… (#582), web: the picker asks for fonts, the picked file arrives through the
/// inbox and is added, listed in the font menu and kept in the browser; removing it forgets it.
#[test]
fn a_font_picked_on_the_web_is_added_listed_and_kept() {
    const FAMILY: &str = "Uifont Sans 3";
    let mut font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16(FAMILY)), (b"Source Sans 3".to_vec(), FAMILY.as_bytes().to_vec())] {
        let mut i = 0;
        while let Some(at) = font[i..].windows(from.len()).position(|w| w == from) {
            font[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    let (mut a, files) = typed_scripted();
    let kept = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(String, usize)>::new()));
    let (k1, k2) = (kept.clone(), kept.clone());
    a.services.store_font = Some(Box::new(move |name, bytes| k1.borrow_mut().push((name.to_string(), bytes.len()))));
    a.services.forget_font = Some(Box::new(move |name| k2.borrow_mut().retain(|(n, _)| n != name)));
    let ctx = crate::test_harness::context();
    assert!(!a.previews.families().iter().any(|f| f == FAMILY));
    a.run("font.add", json!({})).unwrap();
    assert_eq!(files.requests().last().map(|r| r.purpose.as_str()), Some("font"), "the host was asked for a font file");
    drop_files(&mut a, vec![dropped_bytes("Uifont-Regular.ttf", font.clone())]);
    a.drain_inbox();
    a.watch_fonts(&ctx);
    assert!(a.previews.families().iter().any(|f| f == FAMILY), "listed in the font menu at once");
    assert_eq!(*kept.borrow(), vec![("Uifont-Regular.ttf".to_string(), font.len())]);
    assert!(body_text(&a).contains(UNSAVED), "the document stays open");
    // Junk named like a font is refused, and nothing is kept.
    drop_files(&mut a, vec![dropped_bytes("junk.woff2", b"wOF2junk".to_vec())]);
    a.drain_inbox();
    assert_eq!(kept.borrow().len(), 1);
    a.run("font.remove", json!({"family": FAMILY})).unwrap();
    a.watch_fonts(&ctx);
    assert!(kept.borrow().is_empty());
    assert!(!a.previews.families().iter().any(|f| f == FAMILY));
}

/// Issue #859: lifting the pen jumped the view back to the caret, usually on page 1. Drawing
/// leaves the caret where it was, so the view stays on the ink; typing still follows the caret.
#[test]
fn drawing_ink_keeps_the_view_where_it_is() {
    let mut a = typed();
    let keeps_view = |a: &mut WordApp, id: &str, params: Value| {
        a.canvas.scroll_to_caret = false;
        let r = a.run(id, params);
        assert!(r.is_ok(), "{id}: {r:?}");
        assert!(!a.canvas.scroll_to_caret, "{id} scrolled to the caret");
    };
    keeps_view(&mut a, "draw.stroke", json!({"page": 0, "points": [[100, 300], [200, 320]], "tool": "pen"}));
    keeps_view(&mut a, "draw.lasso", json!({"page": 0, "points": [[80, 280], [220, 280], [220, 340], [80, 340]]}));
    keeps_view(&mut a, "draw.lassoMove", json!({"dx": 10, "dy": 10}));
    keeps_view(&mut a, "draw.stroke", json!({"page": 0, "points": [[300, 500], [400, 500]], "tool": "pencil"}));
    keeps_view(&mut a, "draw.erase", json!({"page": 0, "x": 350, "y": 500}));
    a.canvas.scroll_to_caret = false;
    a.run("text.insert", json!({"text": "x"})).unwrap();
    assert!(a.canvas.scroll_to_caret, "typing still brings the caret into view");
}

/// #3864: Save (Ctrl+S, the Save button) jumped the view back to the caret, pages away from
/// where the user was reading. Saving, exporting, printing and the document's properties move
/// neither the caret nor the text, so the view stays, as in Word; opening another document
/// still shows its caret.
#[test]
fn saving_and_exporting_keep_the_view_where_it_is() {
    let dir = scratch("save-keeps-view");
    let path = |name: &str| dir.join(name).to_string_lossy().to_string();
    let mut a = typed();
    let keeps_view = |a: &mut WordApp, id: &str, params: Value| {
        a.canvas.scroll_to_caret = false;
        let r = a.run(id, params);
        assert!(r.is_ok(), "{id}: {r:?}");
        assert!(!a.canvas.scroll_to_caret, "{id} scrolled to the caret");
    };
    keeps_view(&mut a, "file.saveAs", json!({"path": path("report.docx")}));
    a.run("text.insert", json!({"text": "x"})).unwrap();
    keeps_view(&mut a, "file.save", json!({}));
    keeps_view(&mut a, "file.encrypt", json!({"password": "hunter2"}));
    keeps_view(&mut a, "file.exportPdf", json!({"path": path("report.pdf")}));
    keeps_view(&mut a, "file.exportPng", json!({"path": path("report.png")}));
    keeps_view(&mut a, "file.printPdf", json!({"path": path("printed.pdf")}));
    keeps_view(&mut a, "file.saveTemplate", json!({"path": path("report.dotx")}));
    keeps_view(&mut a, "file.print", json!({}));
    keeps_view(&mut a, "file.printMarkup", json!({"value": true}));
    keeps_view(&mut a, "file.properties", json!({"title": "Quarterly report"}));
    keeps_view(&mut a, "file.info", json!({}));
    keeps_view(&mut a, "file.rename", json!({"name": "renamed"}));
    keeps_view(&mut a, "file.privacyOptions", json!({"removeDateTime": true}));
    keeps_view(&mut a, "file.privacy", json!({}));
    keeps_view(&mut a, "file.setAuthor", json!({"name": "Ada"}));
    keeps_view(&mut a, "file.compatibility", json!({}));
    keeps_view(&mut a, "file.share", json!({}));
    keeps_view(&mut a, "file.options", json!({"checkSpelling": false}));
    a.canvas.scroll_to_caret = false;
    a.execute("file.new", json!({})).unwrap();
    assert!(a.canvas.scroll_to_caret, "a new document still shows its caret");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Go to Footer into a footer just made — empty, so the caret at its start also "selects the
/// whole story" — brings the footer into view (it stayed at the header).
#[test]
fn go_to_footer_into_an_empty_footer_scrolls_to_it() {
    let mut a = typed();
    a.run("insert.editHeader", json!({})).unwrap();
    a.run("text.insert", json!({"text": "Umowa nr 7/2026"})).unwrap();
    a.canvas.scroll_to_caret = false;
    a.run("insert.editFooter", json!({})).unwrap();
    assert!(a.session.sel.is_collapsed() && a.session.selects_whole_story(), "the footer is empty");
    assert!(a.canvas.scroll_to_caret, "the view follows the caret into the footer");
}

#[test]
fn select_all_does_not_scroll_to_the_end() {
    let mut a = typed();
    a.canvas.scroll_to_caret = true;
    a.run("select.all", json!({})).unwrap();
    assert!(!a.canvas.scroll_to_caret);
    a.run("format.bold", json!({})).unwrap();
    assert!(!a.canvas.scroll_to_caret);
    a.run("edit.copy", json!({})).unwrap();
    assert!(!a.canvas.scroll_to_caret);
    a.run("text.insert", json!({"text": "x"})).unwrap();
    assert!(a.canvas.scroll_to_caret, "typing still brings the caret into view");
}

/// #1129: with no document open (the Start screen's empty window), `document.inspect` and
/// `ui.render` on the control channel say so instead of reading the hidden placeholder.
#[test]
fn control_inspect_and_render_need_a_document() {
    let ctx = crate::test_harness::context();
    let mut a = app();
    a.start_screen();
    assert!(!a.has_document());
    for (method, params) in [("document.inspect", json!({})), ("ui.render", json!({"page": 1, "scale": 0.5}))] {
        let (req, _reply) = ControlRequest::new(method, params);
        let Reply::Done(v) = control::handle(&mut a, &ctx, &req) else { panic!("{method} replies at once") };
        assert_eq!(v["ok"], false, "{method}: {v}");
        assert!(v["error"].as_str().is_some_and(|e| e.contains("no document is open")), "{method}: {v}");
    }
    a.run("file.new", json!({})).unwrap();
    let (req, _reply) = ControlRequest::new("document.inspect", json!({}));
    let Reply::Done(v) = control::handle(&mut a, &ctx, &req) else { panic!("document.inspect replies at once") };
    assert_eq!(v["ok"], true, "with a document it works: {v}");
}

/// `ui.render` without a `path` returns the PNG in the reply instead of
/// writing a fixed name into the shared temporary folder, where another user could have
/// planted a link to one of this user's files.
#[test]
fn render_without_a_path_replies_with_the_png() {
    let ctx = crate::test_harness::context();
    let mut a = typed();
    let (req, _reply) = ControlRequest::new("ui.render", json!({"page": 1, "scale": 0.5}));
    let Reply::Done(v) = control::handle(&mut a, &ctx, &req) else { panic!("ui.render replies at once") };
    assert_eq!(v["ok"], true, "{v}");
    assert!(v["result"].get("path").is_none(), "no file: {v}");
    let png = wordcraft_engine::cmd::insert::base64_decode(v["result"]["png"].as_str().expect("png")).expect("base64");
    assert_eq!(&png[..4], b"\x89PNG", "{v}");
    assert!(v["result"]["width"].as_u64().unwrap() > 0 && v["result"]["pages"].as_u64().unwrap() >= 1, "{v}");
}

/// #821: Shift+F10 opens the page's context menu below the caret, like right-clicking there
/// (Word's keyboard way to it; the Menu key isn't passed on by the windowing layer); Escape
/// closes it and typing goes on.
#[test]
fn shift_f10_opens_the_context_menu_at_the_caret() {
    use egui_kittest::kittest::Queryable;
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app());
    for _ in 0..6 {
        h.step();
    }
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    h.event(egui::Event::Text("Kilns fire".into()));
    h.step();
    let pastes = |h: &egui_kittest::Harness<WordApp>| h.query_all_by_label_contains("Paste").count();
    let before = pastes(&h);
    h.event(key(egui::Key::F10, egui::Modifiers::SHIFT));
    h.step();
    h.step();
    assert!(h.state().canvas.context_menu_open, "Shift+F10 opened the menu");
    assert!(pastes(&h) > before, "the page's context menu, with Paste");
    let caret = canvas::caret_screen(h.state_mut()).unwrap();
    // The menu's own item stands below the caret, with its left edge at the caret's x plus the menu's padding.
    let item = h
        .query_all_by_label_contains("Paste")
        .map(|node| node.rect())
        .filter(|rect| rect.top() > caret.0.y)
        .min_by(|p, q| p.top().total_cmp(&q.top()))
        .expect("a menu item below the caret");
    assert!(item.left() >= caret.0.x - 1.0 && item.left() < caret.0.x + 40.0, "at the caret's x: {item:?} vs {caret:?}");
    assert!(!h.state().keytips_active, "Shift+F10 doesn't show keytips");
    h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
    h.step();
    h.step();
    assert!(!h.state().canvas.context_menu_open, "Escape closes it");
    // Scripts and agents open it too.
    h.state_mut().run("ui.contextMenu", json!({})).unwrap();
    h.step();
    h.step();
    assert!(h.state().canvas.context_menu_open);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns fire");
}

/// A file dropped in the first two font-warm-up passes is taken before the early return;
/// eframe does not preserve dropped files for a later frame.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_drop_before_font_warm_up_is_not_lost() {
    let mut app = app();
    let path = std::env::temp_dir().join(format!("wordcraft-drop-warmup-{}.txt", std::process::id()));
    std::fs::write(&path, b"dropped text").expect("write the dropped document");
    drop_files(&mut app, vec![DroppedFile { path: Some(path.clone()), name: "warmup.txt".into(), bytes: None }]);
    let mut h = harness(app);
    h.run_steps(4);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end_matches(['\r', '\n']), "dropped text");
    let _ = std::fs::remove_file(&path);
}

/// #3859: a file dropped while a modal dialog is open does nothing: it would insert a picture or
/// open a document behind the dialog, and an opened document closed the dialog.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_drop_behind_a_modal_dialog_does_nothing() {
    let mut app = app();
    app.run("ui.dialog", json!({"name": "font"})).unwrap();
    let path = std::env::temp_dir().join(format!("wordcraft-drop-modal-{}.txt", std::process::id()));
    std::fs::write(&path, b"dropped text").expect("write the dropped document");
    drop_files(&mut app, vec![DroppedFile { path: Some(path.clone()), name: "modal.txt".into(), bytes: None }]);
    let mut h = harness(app);
    h.run_steps(4);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "", "nothing opened behind the dialog");
    assert_eq!(open_dialog(h.state()).map(|d| d.0).as_deref(), Some("font"), "the dialog stays");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn format_painter_single_click_formats_the_target_word_and_finishes() {
    let mut a = app();
    let text = "ORIGEM formatada e DESTINO comum";
    a.run("text.insert", json!({"text": text})).unwrap();
    let mut start = a.session.sel.focus.clone();
    start.off = 0;
    let mut end = start.clone();
    end.off = "ORIGEM".len();
    a.run("caret.set", json!({"pos": start})).unwrap();
    a.run("caret.set", json!({"pos": end, "extend": true})).unwrap();
    for id in ["format.bold", "format.italic", "format.underline"] {
        a.run(id, json!({})).unwrap();
    }
    let source = a.session.doc.para_at(&start).unwrap().props_of_char(0).clone();
    a.run("edit.formatPainter", json!({})).unwrap();
    let mut target = start.clone();
    target.off = text.find("DESTINO").unwrap() + 2;
    let mut h = harness(a);
    h.run_steps(6);
    let caret = h.state_mut().session.layout().caret_on(&target, 0).unwrap();
    let p = canvas::page_to_screen(h.state_mut(), caret.page, caret.x, caret.top + caret.height * 0.5).unwrap();
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    assert!(h.state().session.painter.is_none(), "one click consumes the non-sticky painter");
    let para = h.state().session.doc.para_at(&target).unwrap();
    let got = para.props_of_char(target.off);
    assert_eq!((&got.bold, &got.italic, &got.underline), (&source.bold, &source.italic, &source.underline));
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), text);
    assert_ne!(para.props_of_char(text.find("DESTINO").unwrap() + "DESTINO".len()).bold, source.bold, "trailing space stays plain");
    assert_ne!(para.props_of_char(text.len() - 2).bold, source.bold, "surrounding text stays plain");
}

#[test]
fn format_painter_double_click_drag_and_copy_paste_preserve_target_scope() {
    for gesture in ["double", "drag", "commands"] {
        let mut a = app();
        let text = "ORIGEM formatada e DESTINO comum";
        a.run("text.insert", json!({"text": text})).unwrap();
        let mut source_start = a.session.sel.focus.clone();
        source_start.off = 0;
        let mut source_end = source_start.clone();
        source_end.off = "ORIGEM".len();
        a.run("caret.set", json!({"pos": source_start})).unwrap();
        a.run("caret.set", json!({"pos": source_end, "extend": true})).unwrap();
        for id in ["format.bold", "format.italic", "format.underline"] {
            a.run(id, json!({})).unwrap();
        }
        let source = a.session.doc.para_at(&source_start).unwrap().props_of_char(0).clone();
        let mut target_start = source_start.clone();
        target_start.off = text.find("DESTINO").unwrap();
        let mut target_end = target_start.clone();
        target_end.off += "DESTINO".len();
        a.run(if gesture == "commands" { "edit.copyFormat" } else { "edit.formatPainter" }, json!({})).unwrap();
        let mut h = harness(a);
        h.run_steps(6);
        if gesture == "commands" {
            h.state_mut().run("caret.set", json!({"pos": target_start})).unwrap();
            h.state_mut().run("caret.set", json!({"pos": target_end, "extend": true})).unwrap();
            h.state_mut().run("edit.pasteFormat", json!({})).unwrap();
        } else {
            let mut press = target_start.clone();
            if gesture == "double" {
                press.off += 2;
            }
            let caret = h.state_mut().session.layout().caret_on(&press, 0).unwrap();
            let start = canvas::page_to_screen(h.state_mut(), caret.page, caret.x, caret.top + caret.height * 0.5).unwrap();
            let caret = h.state_mut().session.layout().caret_on(&target_end, 0).unwrap();
            let end = canvas::page_to_screen(h.state_mut(), caret.page, caret.x, caret.top + caret.height * 0.5).unwrap();
            h.event(egui::Event::PointerMoved(start));
            h.run_steps(1);
            let clicks = if gesture == "double" { 2 } else { 1 };
            for _ in 0..clicks {
                h.event(egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                });
                h.run_steps(1);
                let release = if gesture == "drag" {
                    h.event(egui::Event::PointerMoved(end));
                    h.run_steps(2);
                    end
                } else {
                    start
                };
                h.event(egui::Event::PointerButton {
                    pos: release,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
                h.run_steps(1);
            }
        }
        assert!(h.state().session.painter.is_none(), "{gesture}: one-use painter must disarm");
        let para = h.state().session.doc.para_at(&target_start).unwrap();
        for off in target_start.off..target_end.off {
            let got = para.props_of_char(off);
            assert_eq!((&got.bold, &got.italic, &got.underline), (&source.bold, &source.italic, &source.underline), "{gesture}: target offset {off}");
        }
        assert_ne!(para.props_of_char(target_end.off).bold, source.bold, "{gesture}: trailing space stays plain");
        assert_ne!(para.props_of_char(text.len() - 2).bold, source.bold, "{gesture}: surrounding text stays plain");
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), text);
    }
}

/// Issue #6146: touchpad scrolling on Linux/Wayland is scaled by `touchpad_factor` in
/// `raw_input_hook` so swiping matches the desktop's speed instead of crawling 1:1.
/// Wheel notches scale with `ui.scrollSpeed`. Both settings persist in `ui.json` and are set on
/// `ui.touchpadScrollSpeed` and `ui.scrollSpeed`.
#[test]
fn touchpad_and_mouse_scroll_speed_scaling_and_preferences() {
    use crate::scroll::ScrollSpeed;
    let mut a = app();
    // The factors reach the frame through `root_env().wheel` (sh.A1); the runtime scales the input.
    let point = |a: &WordApp| a.root_env().wheel.point;
    let line = |a: &WordApp| a.root_env().wheel.line;
    let touchpad = |speed: ScrollSpeed| if cfg!(target_arch = "wasm32") { speed.wheel_factor() } else { speed.touchpad_factor() };
    assert_eq!(point(&a), touchpad(ScrollSpeed::Normal), "point deltas are scaled by the touchpad factor");
    assert_eq!(line(&a), ScrollSpeed::Normal.wheel_factor());

    a.ui.touchpad_scroll_speed = ScrollSpeed::Fast;
    assert_eq!(point(&a), touchpad(ScrollSpeed::Fast), "the Fast touchpad factor reaches the frame");
    assert_eq!(line(&a), ScrollSpeed::Normal.wheel_factor(), "notches keep their own setting");

    // 2. Commands ui.touchpadScrollSpeed and ui.scrollSpeed set the preferences.
    let r = a.run("ui.touchpadScrollSpeed", json!({"value": "fast"})).unwrap();
    assert_eq!(r["touchpadScrollSpeed"], "fast");
    assert_eq!(a.ui.touchpad_scroll_speed, crate::scroll::ScrollSpeed::Fast);

    let r = a.run("ui.scrollSpeed", json!({"value": "fast"})).unwrap();
    assert_eq!(r["scrollSpeed"], "fast");
    assert_eq!(a.ui.scroll_speed, crate::scroll::ScrollSpeed::Fast);

    assert!(a.run("ui.touchpadScrollSpeed", json!({"value": "warp"})).is_err());
    assert!(a.run("ui.scrollSpeed", json!({"value": "warp"})).is_err());

    // 3. Persistence between runs, through `Prefs` (sh.A13).
    let mut restored = app();
    restored.apply_prefs(a.prefs());
    assert_eq!(restored.ui.touchpad_scroll_speed, ScrollSpeed::Fast);
    assert_eq!(restored.ui.scroll_speed, ScrollSpeed::Fast);
}

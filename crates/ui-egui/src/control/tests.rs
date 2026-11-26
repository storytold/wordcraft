//! Control-channel tests that stay with the app: the write authority, the close guard, `ui.clickText`,
//! `ui.closeWindow`, `ui.clipboard`, `ui.resize`, the warm-up gate and keyboard delivery.

use super::*;
use craft::automation::control::{ControlChannel, ControlRequest, Reply};

/// A context with the framework runtime installed, as the app's host installs it.
fn installed() -> egui::Context {
    crate::test_harness::context()
}

/// The app in a headless harness that runs `logic` and `ui` and installs the runtime.
fn harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, a: &mut WordApp| {
            let ctx = ui.ctx().clone();
            crate::test_harness::logic(&ctx, a);
            a.ui(ui);
        },
        app,
    );
    frames(&mut h, 6);
    h
}

/// `n` frames, each given the input the app hands the runtime.
fn frames(h: &mut egui_kittest::Harness<'_, WordApp>, n: usize) {
    for _ in 0..n {
        let ctx = h.ctx.clone();
        craft::ui::shell::embed::before_input(&ctx, |out| h.state_mut().frame_input(out));
        h.step();
    }
}

/// One request through `handle`; input replies stand for "queued".
fn send(h: &mut egui_kittest::Harness<'_, WordApp>, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    match control::handle(h.state_mut(), &ctx, &req) {
        Reply::Done(v) => v,
        Reply::AfterInput => json!({"ok": true, "result": {"queued": true}}),
        _ => json!({"ok": false, "error": "not a reply"}),
    }
}

/// One request against an app with no frames behind it.
fn call(a: &mut WordApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match control::handle(a, ctx, &req) {
        Reply::Done(v) => v,
        _ => json!({"ok": false, "error": "not a reply"}),
    }
}

struct Cleanup(std::path::PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A screenshot path must end in `.png` and is refused otherwise; a PNG is written, a symlinked
/// target is not followed, and a path outside the automation roots is refused.
#[test]
fn automation_files_are_png_only_inside_the_roots_and_never_follow_links() {
    let dir = std::env::temp_dir().join(format!("wc-auto-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = Cleanup(dir.clone());
    let png_bytes: &[u8] = b"\x89PNG\r\n\x1a\nbody";

    let txt = dir.join("shot.txt");
    assert!(control::write_automation_file(txt.to_str().unwrap(), png_bytes).is_err());
    assert!(!txt.exists());

    let png = dir.join("shot.png");
    control::write_automation_file(png.to_str().unwrap(), png_bytes).unwrap();
    assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));

    #[cfg(unix)]
    {
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, "keep").unwrap();
        let link = dir.join("link.png");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        let _ = control::write_automation_file(link.to_str().unwrap(), png_bytes);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    }

    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let roots = wordcraft_engine::file_access::AutomationRoots::new(None, Some(&other)).unwrap();
    let outside = dir.join("outside.png");
    let r = wordcraft_engine::file_access::confine(roots.as_ref(), || control::write_automation_file(outside.to_str().unwrap(), png_bytes));
    assert!(r.is_err());
    assert!(!outside.exists());
}

/// The request for one way to close, with `force` as given (absent, or any JSON value).
fn closing(form: &str, force: Option<&Value>, target: u64, background: bool) -> (&'static str, Value) {
    let mut params = serde_json::Map::new();
    if let Some(f) = force {
        params.insert("force".into(), f.clone());
    }
    let doc = |mut v: serde_json::Map<String, Value>| {
        if background {
            v.insert("doc".into(), json!(target));
        }
        Value::Object(v)
    };
    match form {
        "engine.execute" => (
            "engine.execute",
            doc([("command".to_string(), json!("file.close")), ("params".to_string(), Value::Object(params))].into_iter().collect()),
        ),
        "command" => ("command", doc([("id".to_string(), json!("file.close")), ("params".to_string(), Value::Object(params))].into_iter().collect())),
        "engine.execute:closeDocument" => {
            params.insert("id".into(), json!(target));
            (
                "engine.execute",
                doc([("command".to_string(), json!("file.closeDocument")), ("params".to_string(), Value::Object(params))].into_iter().collect()),
            )
        }
        "file.closeDocument" => {
            params.insert("id".into(), json!(target));
            ("file.closeDocument", Value::Object(params))
        }
        "app.quit" => ("app.quit", Value::Object(params)),
        _ => ("file.close", Value::Object(params)),
    }
}

/// Every way to close or quit over the control channel keeps unsaved work unless `force` is the
/// boolean true, for the active document and for one in the background (#1268, #3274).
#[test]
fn programmatic_close_guard_covers_every_form() {
    let ctx = crate::test_harness::context();
    let forces: [(Option<Value>, bool); 4] = [(None, false), (Some(json!("yes")), false), (Some(json!(1)), false), (Some(json!(true)), true)];
    let forms = ["engine.execute", "command", "file.close", "engine.execute:closeDocument", "file.closeDocument", "app.quit"];
    for form in forms {
        for background in [false, true] {
            // A direct `file.close` method has no `doc`.
            if background && form == "file.close" {
                continue;
            }
            for (force, accepted) in &forces {
                let mut a = typed();
                let first = a.session.document_id();
                if background {
                    a.run("file.new", json!({})).unwrap();
                }
                let (method, params) = closing(form, force.as_ref(), first, background);
                let r = call(&mut a, &ctx, method, params.clone());
                let what = format!("{form} background={background} {params}: {r}");
                assert_eq!(r["ok"], json!(*accepted), "{what}");
                if *accepted {
                    if form == "app.quit" {
                        assert!(a.quit_requested, "{what}");
                    } else {
                        assert!(!a.session.is_open(first), "{what}");
                    }
                } else {
                    assert!(!a.quit_requested && a.session.is_open(first), "{what}");
                }
            }
        }
    }
}

/// #1315: a save the browser's picker hasn't finished marks the document saved, yet closing or
/// quitting over the control channel still keeps it.
#[test]
fn programmatic_close_keeps_a_save_the_picker_hasnt_finished() {
    let ctx = crate::test_harness::context();
    for (method, params) in [("engine.execute", json!({"command": "file.close"})), ("file.close", json!({})), ("app.quit", json!({}))] {
        let (mut a, saves) = web_picker_app();
        a.run("file.close", json!({})).unwrap();
        let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(r["pending"], "save", "{r}");
        assert_eq!(saves.pending(), 1);
        let id = a.session.document_id();
        let r = call(&mut a, &ctx, method, params.clone());
        assert_eq!(r["ok"], false, "{method} {params}: {r}");
        assert!(a.session.is_open(id) && !a.quit_requested, "{method}: the document stays");
    }
}

/// `ui.resize` refuses sizes that aren't positive numbers, clamps a huge one to what the surface
/// can be, and asks the window for the size it replies with.
#[test]
fn control_resize_rejects_bad_sizes_and_clamps_huge_ones() {
    let ctx = installed();
    let mut a = app();
    for bad in [json!({}), json!({"width": 0, "height": 100}), json!({"width": -5, "height": 100}), json!({"width": "wide", "height": 100})] {
        let r = call(&mut a, &ctx, "ui.resize", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    let r = call(&mut a, &ctx, "ui.resize", json!({"width": 1.0e9, "height": 1.0e9}));
    assert_eq!(r["ok"], true, "{r}");
    let max_side = f64::from(ctx.input(|i| i.max_texture_side) as f32 / ctx.pixels_per_point());
    assert!(r["result"]["width"].as_f64().unwrap() <= max_side, "{r}");
    assert!(r["result"]["height"].as_f64().unwrap() <= max_side, "{r}");

    let out = ctx.run_ui(egui::RawInput::default(), |_| {});
    let asked =
        out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::InnerSize(_))));
    out.drop_without_applying_deltas();
    assert!(asked, "the window was asked for the size");
}

/// `ui.clickText` clicks the screen point a page point is shown at, and refuses a page that isn't
/// on screen.
#[test]
fn ui_clicktext_clicks_the_page_point_it_names() {
    let mut a = app();
    a.run("document.setText", json!({"text": "Seed"})).unwrap();
    let mut h = harness(a);
    h.state_mut().run("caret.docEnd", json!({})).unwrap();
    frames(&mut h, 2);
    let before = h.state().session.sel.focus.clone();
    let r = send(&mut h, "ui.clickText", json!({"page": 0, "x": 72.5, "y": 80.0}));
    assert_eq!(r["ok"], true, "{r}");
    let screen = r["result"]["screen"].as_array().unwrap();
    let page = h.state().canvas.page_rects().next().unwrap().1;
    assert!(page.contains(egui::pos2(screen[0].as_f64().unwrap() as f32, screen[1].as_f64().unwrap() as f32)), "{r}");
    frames(&mut h, 8);
    assert_ne!(h.state().session.sel.focus, before, "the caret moved to the clicked word");

    let r = send(&mut h, "ui.clickText", json!({"page": 99, "x": 1.0, "y": 1.0}));
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap().contains("page not on screen"), "{r}");
}

/// `ui.closeWindow` is the OS close button: unsaved work holds the close behind the Save prompt;
/// with none the close goes ahead.
#[test]
fn ui_closewindow_asks_the_close_guard_like_the_os_button() {
    let mut h = harness(typed());
    let r = send(&mut h, "ui.closeWindow", json!({}));
    assert_eq!(r["ok"], true, "{r}");
    // The window host turns the request into a viewport Close command; a kittest harness has no OS window to
    // answer it, so the answer is delivered as eframe delivers it: the root viewport reports a close request.
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    frames(&mut h, 6);
    assert_eq!(prompt(h.state()), Some("saveChanges"), "the guard holds the close");
    assert!(h.state().has_document() && !h.state().quit_requested);

    let mut h = harness(app());
    let r = send(&mut h, "ui.closeWindow", json!({}));
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h, 6);
    assert_eq!(prompt(h.state()), None);
    assert!(h.state_mut().close_requested(), "nothing to ask: the close is raised");
}

/// `ui.clipboard` writes the flavours it is given; the others stay absent.
#[test]
fn ui_clipboard_sets_each_flavour_it_is_given() {
    use craft::platform::clipboard::{Clipboard, MemoryClipboard};
    use craft::ui::runtime::commands::ClipboardHandle;
    let ctx = crate::test_harness::context_with(|| crate::test_harness::install_spec().clipboard(Clipboard::new(MemoryClipboard::default())));
    let mut a = app();
    let r = call(&mut a, &ctx, "ui.clipboard", json!({"text": "plain", "html": "<b>rich</b>"}));
    assert_eq!(r["ok"], true, "{r}");
    let clip = ClipboardHandle::of(&ctx);
    assert_eq!(clip.read_text().unwrap().as_deref(), Some("plain"));
    assert!(clip.has(&["text/html"]));
    assert!(!clip.has(&["text/rtf"]), "a flavour not given is absent");
    let r = call(&mut a, &ctx, "ui.clipboard", json!({}));
    assert_eq!(r["ok"], false, "{r}");
}

/// A queued request stays in the channel until the font warm-up frames have passed, rather than
/// running against a layout with no usable font atlas.
#[test]
fn synthetic_input_waits_for_font_warm_up() {
    let ctx = installed();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut a = app().with_control(ControlChannel::new(rx));
    a.fonts_frames = 0;
    let (req, reply) = ControlRequest::new("ui.inspect", json!({}));
    tx.send(req).unwrap();
    a.control_logic(&ctx);
    assert!(reply.try_recv().is_err(), "the request waits for the warm-up");
    a.fonts_frames = 2;
    a.control_logic(&ctx);
    assert!(reply.try_recv().is_ok(), "answered once the fonts are warm");
}

/// #2420: control text reaches the document once, as one undo step, and foregrounding never replays it.
#[test]
fn control_keyboard_reaches_the_background_document_once() {
    let mut a = app();
    a.run("document.setText", json!({"text": "Seed"})).unwrap();
    let mut h = harness(a);
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    frames(&mut h, 4);
    h.state_mut().run("select.all", json!({})).unwrap();
    send(&mut h, "ui.type", json!({"text": "Written"}));
    frames(&mut h, 8);
    assert_eq!(body_text(h.state()), "Written");
    let rev = h.state().session.rev();
    frames(&mut h, 6);
    assert_eq!(h.state().session.rev(), rev, "no input is replayed");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(body_text(h.state()), "Seed", "one Undo reverses the one text input");
}

/// Control text and keys target the focused widget of a dialog, not the document. The Find dialog is modeless, so the
/// first Escape reverts the field's typing and the second closes the dialog.
#[test]
fn control_keyboard_keeps_the_background_widget_target() {
    let mut a = app();
    a.run("document.setText", json!({"text": "Seed"})).unwrap();
    let mut h = harness(a);
    h.state_mut().run("edit.advancedFind", json!({})).unwrap();
    frames(&mut h, 4);
    let open = |h: &mut egui_kittest::Harness<'_, WordApp>| {
        let r = send(h, "ui.inspect", json!({}));
        r["result"]["dialogs"].as_array().map_or(0, Vec::len)
    };
    assert!(open(&mut h) > 0, "the Find dialog is open");
    send(&mut h, "ui.type", json!({"text": "Needle"}));
    frames(&mut h, 6);
    assert_eq!(body_text(h.state()), "Seed", "the Find field, not the document, owns the input");
    send(&mut h, "ui.key", json!({"key": "Escape"}));
    frames(&mut h, 6);
    assert!(open(&mut h) > 0, "the first Escape reverts the field and leaves the dialog open");
    send(&mut h, "ui.key", json!({"key": "Escape"}));
    frames(&mut h, 6);
    assert_eq!(open(&mut h), 0, "the second Escape closes the dialog");
    assert_eq!(body_text(h.state()), "Seed");
}

/// Control input respects read-only protection like real keys.
#[test]
fn control_keyboard_preserves_native_focus_and_document_protection() {
    let mut a = app();
    a.run("document.setText", json!({"text": "Seed"})).unwrap();
    let mut h = harness(a);
    h.state_mut().run("select.all", json!({})).unwrap();
    h.state_mut().run("review.restrict", json!({"mode": "readOnly"})).unwrap();
    let doc = h.state().session.doc.clone();
    frames(&mut h, 2);
    send(&mut h, "ui.type", json!({"text": "Refused"}));
    frames(&mut h, 8);
    assert_eq!(h.state().session.doc, doc, "control input respects the same protection as real keys");
}

/// A script's `engine.execute` of a command that only asks for its dialog opens it, as production did (`format.fontDialog`
/// opened the Font dialog); a command that changes the document and lacked its input (Insert Icon without an `id`) opens
/// nothing and gives the script its data.
#[test]
fn a_script_opens_the_dialog_a_launcher_command_asks_for() {
    let ctx = installed();
    let fresh = || WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let mut a = fresh();
    let r = call(&mut a, &ctx, "engine.execute", json!({"command": "format.fontDialog"}));
    assert_eq!(r["ok"], json!(true), "{r}");
    let keys: Vec<String> = a.dialog_host.open_dialogs().into_iter().filter_map(|d| d.key).collect();
    assert!(keys.iter().any(|k| k == "font"), "the Font dialog is open: {keys:?}");
    let mut b = fresh();
    let r = call(&mut b, &ctx, "engine.execute", json!({"command": "insert.icon"}));
    assert!(r["result"]["icons"].is_array(), "{r}");
    assert!(b.dialog_host.open_dialogs().is_empty(), "a script's Insert Icon without an id opens no picker");
}

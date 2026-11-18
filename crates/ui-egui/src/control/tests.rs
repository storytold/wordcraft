use super::*;

/// #2420: an inactive native window still delivers control input to its focused widget.
#[test]
fn control_keyboard_reaches_the_background_document_once() {
    for focused in [true, false] {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("document.setText", json!({"text": "Seed"})).unwrap();
        for _ in 0..4 {
            control_frame(&mut a, &ctx, true);
        }
        assert!(a.canvas.focused);
        for _ in 0..2 {
            control_frame(&mut a, &ctx, focused);
        }
        a.run("select.all", json!({})).unwrap();
        // Two queued requests are delivered in order across the key's release boundary.
        control_input(&mut a, &ctx, "ui.key", json!({"key": "b", "cmd": true}));
        control_input(&mut a, &ctx, "ui.text", json!({"text": "Written"}));
        control_frame(&mut a, &ctx, focused);
        assert_eq!(a.session.doc.para_at(&a.session.sel.focus).unwrap().props_of_char(0).bold, Some(true), "focused={focused}: Mod+B applied");
        assert_eq!(body_text(&a), "Seed", "text waits behind the key's release");
        control_frame(&mut a, &ctx, focused);
        assert_eq!(body_text(&a), "Written", "focused={focused}: text applied");
        assert!(a.synthetic.is_empty());
        let rev = a.session.rev();
        for _ in 0..3 {
            control_frame(&mut a, &ctx, true);
        }
        assert_eq!(a.session.rev(), rev, "foregrounding never replays an input");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body_text(&a), "Seed", "one Undo reverses the one text input");
    }
}

/// Synthetic control text and keys target the focused background widget (the Find field), and
/// Escape closes it. Closing a dirty document offers Save Changes, and Escape cancels without
/// losing the document.
#[test]
fn control_keyboard_keeps_the_background_widget_target() {
    let ctx = egui::Context::default();
    let mut a = app();
    a.run("document.setText", json!({"text": "Seed"})).unwrap();
    for _ in 0..4 {
        control_frame(&mut a, &ctx, true);
    }
    a.run("edit.advancedFind", json!({})).unwrap();
    for _ in 0..2 {
        control_frame(&mut a, &ctx, true);
    }
    assert!(ctx.memory(|m| m.has_focus(egui::Id::unique("find_what"))));
    for _ in 0..2 {
        control_frame(&mut a, &ctx, false);
    }
    control_input(&mut a, &ctx, "ui.text", json!({"text": "Needle"}));
    control_frame(&mut a, &ctx, false);
    assert!(matches!(&a.dialog, Some(dialogs::Dialog::Find { query, .. }) if query == "Needle"));
    control_input(&mut a, &ctx, "ui.key", json!({"key": "Backspace"}));
    control_frame(&mut a, &ctx, false);
    assert!(matches!(&a.dialog, Some(dialogs::Dialog::Find { query, .. }) if query == "Needl"));
    assert_eq!(body_text(&a), "Seed", "the Find field, not the document, owns the input");
    control_input(&mut a, &ctx, "ui.key", json!({"key": "Escape"}));
    control_frame(&mut a, &ctx, false);
    assert!(a.dialog.is_none());
    a.run("file.close", json!({})).unwrap();
    for focused in [true, true, false, false] {
        control_frame(&mut a, &ctx, focused);
    }
    assert!(matches!(a.dialog, Some(dialogs::Dialog::SaveChanges { .. })));
    control_input(&mut a, &ctx, "ui.key", json!({"key": "Escape"}));
    control_frame(&mut a, &ctx, false);
    assert!(a.dialog.is_none());
    assert!(a.session.dirty && a.has_document() && !a.quit_requested);
    assert_eq!(body_text(&a), "Seed", "Cancel preserves the unsaved document");
}

/// Control input respects read-only protection, and a frame without control keys leaves the native
/// focus state alone.
#[test]
fn control_keyboard_preserves_native_focus_and_document_protection() {
    let ctx = egui::Context::default();
    let mut a = app();
    a.run("document.setText", json!({"text": "Seed"})).unwrap();
    for _ in 0..4 {
        control_frame(&mut a, &ctx, true);
    }
    a.run("select.all", json!({})).unwrap();
    a.run("review.restrict", json!({"mode": "readOnly"})).unwrap();
    let doc = a.session.doc.clone();
    for _ in 0..2 {
        control_frame(&mut a, &ctx, false);
    }
    control_input(&mut a, &ctx, "ui.key", json!({"key": "b", "cmd": true}));
    control_input(&mut a, &ctx, "ui.text", json!({"text": "Refused"}));
    for _ in 0..3 {
        control_frame(&mut a, &ctx, false);
    }
    assert_eq!(a.session.doc, doc, "control input respects the same protection as real keys");
    assert!(a.synthetic.is_empty());
    let mut native = egui::RawInput { focused: false, ..Default::default() };
    a.raw_input_hook(&ctx, &mut native);
    assert!(!native.focused, "a frame without control keys keeps the native focus state");
    assert!(!a.canvas.focused);
}

/// Every way to name `file.close` over the control channel is guarded (#1268).
#[test]
fn programmatic_close_guard_covers_every_form() {
    let ctx = egui::Context::default();
    let send = |a: &mut WordApp, method: &str, params: Value| match control::handle(a, &ctx, &ControlRequest::new(method, params).0) {
        control::Outcome::Done(v) => v,
        _ => json!({"ok": false, "error": "not a reply"}),
    };
    let refused = [
        ("file.close", json!({})),
        ("command", json!({"id": "file.close"})),
        ("command", json!({"command": "file.close", "params": {"force": "yes"}})),
        ("file.close", json!({"force": "yes"})),
        ("file.close", json!({"force": 1})),
    ];
    for (method, params) in refused {
        let mut a = typed();
        let r = send(&mut a, method, params.clone());
        assert_eq!(r["ok"], json!(false), "{method} {params}: {r}");
        assert!(a.has_document() && !a.quit_requested, "{method} {params}");
        assert!(body_text(&a).contains(UNSAVED));
    }
    let mut a = typed();
    let r = send(&mut a, "file.close", json!({"force": true}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert!(!a.has_document() && !a.quit_requested);
}

/// A synthetic step waits for the font warm-up frames rather than being injected into a pass
/// whose layout has no usable font atlas.
#[test]
fn synthetic_input_waits_for_font_warm_up() {
    let mut app = app();
    app.synthetic = vec![egui::Event::Text("x".into())];
    let ctx = egui::Context::default();
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut raw);
    assert!(raw.events.is_empty(), "fonts are not ready yet");
    assert_eq!(app.synthetic.len(), 1, "the step is retained");
}

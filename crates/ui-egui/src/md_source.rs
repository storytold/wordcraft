//! View › Markdown Source: the text pane that takes the page's place. The text itself and what it
//! means for the document are the engine's (`wordcraft_engine::cmd::md_source`).

use egui::Ui;

use craft::behavior::{EditEvent, EditEvents};
use craft::platform::clock::monotonic_ms;
use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, Props as _, Show as _, Sizing};
use craft::ui::notices::Notice;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::runtime::frame::request_repaint_after;
use craft::ui::text::TextField;

use crate::WordApp;

/// How often a text with edits reaches the document while it is being typed, in milliseconds, so
/// crash recovery has them.
const APPLY_EVERY_MS: f64 = 5000.0;

/// The text the field held when the pass began, from the edit events that carry it.
fn text_before_pass(events: &EditEvents<String>) -> Option<&String> {
    events.iter().find_map(|event| match event {
        EditEvent::Begin { before, .. } | EditEvent::Set { before, .. } | EditEvent::Commit { before, .. } => before.same(),
        EditEvent::Cancel { before: Some(before), .. } => before.same(),
        _ => None,
    })
}

/// Draw the pane in the place of the page.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let now = monotonic_ms();
    if app.session.md_source.as_ref().is_some_and(|src| src.is_pending() && now - src.applied_at_ms > APPLY_EVERY_MS) {
        wordcraft_engine::cmd::md_source::apply(&mut app.session);
        if let Some(src) = app.session.md_source.as_mut() {
            src.applied_at_ms = now;
        }
    }
    let max = wordcraft_engine::cmd::md_source::MAX_SOURCE_BYTES;
    let mut changed = false;
    let mut refused = false;
    let mut focus_pending = false;
    {
        let Some(src) = app.session.md_source.as_mut() else { return };
        let mut entering = std::mem::take(&mut src.place_caret);
        // A fresh field identity per visit is a fresh typing history.
        if std::mem::take(&mut src.reset_undo) {
            app.md_source_visit = app.md_source_visit.wrapping_add(1);
            entering = true;
        }
        if src.loses_content {
            Notice::new(
                Tone::Neutral,
                tl!("This document has content Markdown can't hold. Changing the text here removes it; one Undo afterwards brings it back."),
            )
            .show(ui);
        }
        // The caret this pass began with, for the refusal below.
        let caret_before = src.caret;
        // A multi-line field: Enter breaks lines, and it holds no character limit. Tab types a tab, which
        // Markdown code blocks use.
        let out = TextField::new(tl!("Markdown Source"), Bind::bound(&mut src.text))
            .placeholder(tl!("Markdown Source"))
            .mono()
            .lines(2)
            .tab_inserts(true)
            .caret(Bind::bound(&mut src.caret))
            .sizing(Sizing::Fill)
            .id_salt(("md-source", app.md_source_visit))
            .show(ui);
        // The field limits by characters and the engine's limit is in bytes, so the refusal is ours.
        if src.text.len() > max {
            match text_before_pass(&out.events) {
                Some(before) => src.text.clone_from(before),
                None => {
                    // The events carry no earlier text (a gesture began in an earlier pass): cut at the limit.
                    let mut end = max;
                    while !src.text.is_char_boundary(end) {
                        end -= 1;
                    }
                    src.text.truncate(end);
                }
            }
            src.caret = caret_before.min(src.text.len());
            src.place_caret = true;
            refused = true;
        } else {
            changed = !out.events.is_empty();
        }
        // A measuring pass moves no focus (`request` says so): the entry waits for a pass that does.
        if entering && !focus::request(&ctx, FocusTarget::Id(out.response.id)) {
            focus_pending = true;
        }
    }
    if focus_pending && let Some(src) = app.session.md_source.as_mut() {
        src.place_caret = true;
    }
    if refused {
        app.status(tl!("Markdown Source takes up to 2 MB of text"));
    }
    if changed {
        app.session.dirty = true;
        request_repaint_after(&ctx, APPLY_EVERY_MS / 1000.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    /// Builds a test harness for Markdown Source UI integration tests.
    ///
    /// Frames are stepped with the input the app hands the runtime (`test_harness::frames`), as eframe does, so the
    /// field's Undo state and the command catalog reach the board. The frame the builder runs has no such input, so a
    /// pane already open is entered again afterwards: its focus request lands on a frame that has it.
    fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        if let Some(src) = h.state_mut().session.md_source.as_mut() {
            src.place_caret = true;
        }
        crate::test_harness::frames(&mut h, 2);
        h
    }

    /// A saved Markdown file, with its folder for the test to remove.
    fn opened(name: &str) -> (WordApp, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "wordcraft-ui-md-source-{name}-{}-{}-{}",
            std::process::id(),
            id,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "# Heading\n\nSome text.\n").unwrap();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        (a, dir)
    }

    /// Whether the board resolves the edit command `id` as enabled for the focused field.
    fn enabled(ctx: &egui::Context, id: &str) -> bool {
        craft::ui::runtime::commands::resolve(ctx, id).enabled
    }

    /// Verifies Markdown Source pane displays text and captures keyboard focus.
    #[test]
    fn the_pane_shows_the_text_and_takes_the_keyboard() {
        let (mut a, dir) = opened("keys");
        a.run("view.markdownSource", json!({})).unwrap();
        let mut h = harness(a);
        crate::test_harness::frames(&mut h, 1);
        crate::test_harness::frames(&mut h, 1);
        let field = h.get_by_label("Markdown Source");
        assert!(field.value().is_some_and(|v| v.contains("# Heading\n\nSome text.")), "the Markdown is shown");
        assert!(field.is_focused(), "the field has the keyboard");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Verifies typing in source view marks document dirty and Ctrl+S saves it.
    #[test]
    fn typing_is_an_unsaved_change_and_ctrl_s_saves_it() {
        let (mut a, dir) = opened("save");
        a.run("view.markdownSource", json!({})).unwrap();
        let mut h = harness(a);
        crate::test_harness::frames(&mut h, 1);
        crate::test_harness::frames(&mut h, 1);
        h.event(egui::Event::Text("Typed. ".into()));
        crate::test_harness::frames(&mut h, 2);
        assert!(h.state().session.dirty, "typing is an unsaved change");
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
        // The key's command reaches the app in the next frame's input hook (`frame_input`), so it takes a second frame.
        crate::test_harness::frames(&mut h, 2);
        let saved = std::fs::read_to_string(dir.join("note.md")).unwrap();
        assert!(saved.contains("Typed."), "{saved}");
        assert!(!h.state().session.dirty);
        assert!(h.state().session.md_source.is_some(), "saving keeps the pane");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Verifies the Quick Access Toolbar's Undo and Redo reach the field's typing history.
    #[test]
    fn the_arrows_undo_and_redo_typing_in_the_pane() {
        let (mut a, dir) = opened("arrows");
        a.run("view.markdownSource", json!({})).unwrap();
        let mut h = harness(a);
        crate::test_harness::frames(&mut h, 1);
        crate::test_harness::frames(&mut h, 1);
        assert!(!enabled(&h.ctx, "edit.undo"), "nothing typed yet");
        h.event(egui::Event::Text("Typed. ".into()));
        // The field's Undo state reaches the board at the next frame's input hook, so it takes a second frame.
        crate::test_harness::frames(&mut h, 2);
        let typed = h.state().session.md_source.as_ref().unwrap().text.clone();
        assert!(typed.contains("Typed. "), "{typed}");
        assert!(enabled(&h.ctx, "edit.undo"));
        h.get_by_label("Undo").click();
        crate::test_harness::frames(&mut h, 3);
        assert_eq!(h.state().session.md_source.as_ref().unwrap().text, "# Heading\n\nSome text.\n", "Undo takes the typing back");
        assert!(h.state().session.md_source.is_some(), "and stays in the pane");
        h.get_by_label("Redo").click();
        crate::test_harness::frames(&mut h, 3);
        assert_eq!(h.state().session.md_source.as_ref().unwrap().text, typed, "Redo puts it back");
        assert!(!enabled(&h.ctx, "edit.redo"), "nothing more to redo");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Verifies rewrite warning stays displayed for twenty seconds on the status bar.
    #[test]
    fn the_rewrite_warning_stays_longer_than_other_messages() {
        let (mut a, dir) = opened("warning");
        a.run("text.insert", json!({"text": "Formatted edit. "})).unwrap();
        a.run("file.save", json!({})).unwrap();
        let status = a.status.as_ref().unwrap();
        assert!(status.text.contains("writing the Markdown out again"), "{}", status.text);
        let left = status.until - monotonic_ms();
        assert!(left > 15_000.0, "shown for about 20 seconds, {left} ms left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Verifies formatting ribbon controls are greyed out while Markdown Source pane is active.
    #[test]
    fn the_ribbon_greys_out_edits_while_the_pane_is_open() {
        let (mut a, dir) = opened("grey");
        assert_eq!(a.session.why_disabled("format.bold"), None);
        a.run("view.markdownSource", json!({})).unwrap();
        assert_eq!(a.session.why_disabled("format.bold"), Some("leave Markdown Source first"));
        assert_eq!(a.session.why_disabled("view.markdownSource"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Verifies that typing or pasting beyond `MAX_SOURCE_BYTES` is refused, keeping the previous text and warning.
    #[test]
    fn text_exceeding_max_source_bytes_is_refused_and_warns() {
        let (mut a, dir) = opened("oversize");
        a.run("view.markdownSource", json!({})).unwrap();
        let limit = wordcraft_engine::cmd::md_source::MAX_SOURCE_BYTES;
        let half = limit / 2;
        let original = format!("START {} END", "é".repeat(half - 10));
        let original_len = original.len();
        a.session.md_source.as_mut().unwrap().text = original.clone();
        let mut h = harness(a);
        crate::test_harness::frames(&mut h, 1);
        crate::test_harness::frames(&mut h, 1);
        h.event(egui::Event::Text("é".repeat(100)));
        crate::test_harness::frames(&mut h, 1);
        let current_text = h.state().session.md_source.as_ref().unwrap().text.clone();
        assert_eq!(current_text, original, "over-limit text must be refused without truncating document end");
        assert_eq!(current_text.len(), original_len);
        // PLAN: assumed (app-hub sh.A3) `status: Option<Status>` with `text`
        let status = h.state().status.as_ref().map(|s| s.text.as_str());
        assert_eq!(status, Some("Markdown Source takes up to 2 MB of text"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Verifies that pointer movement alone does not take a snapshot or alter Markdown Source state.
    #[test]
    fn pointer_movement_does_not_mutate_or_affect_markdown_source() {
        let (mut a, dir) = opened("pointer");
        a.run("view.markdownSource", json!({})).unwrap();
        let mut h = harness(a);
        crate::test_harness::frames(&mut h, 1);
        let before_text = h.state().session.md_source.as_ref().unwrap().text.clone();
        h.event(egui::Event::PointerMoved(egui::pos2(100.0, 200.0)));
        crate::test_harness::frames(&mut h, 1);
        assert_eq!(h.state().session.md_source.as_ref().unwrap().text, before_text);
        assert!(!h.state().session.dirty);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Verifies that reopening Markdown Source after formatted edits starts a fresh typing history, so the
    /// previous visit's edits cannot be restored by Undo.
    #[test]
    fn reopening_pane_resets_undo_history_from_previous_visit() {
        let (mut a, dir) = opened("reopen_undo");
        a.run("view.markdownSource", json!({})).unwrap();
        let mut h = harness(a);
        crate::test_harness::frames(&mut h, 1);
        crate::test_harness::frames(&mut h, 1);
        h.event(egui::Event::Text("First visit. ".into()));
        crate::test_harness::frames(&mut h, 2);
        assert!(enabled(&h.ctx, "edit.undo"), "undo enabled after typing");

        h.state_mut().run("view.markdownSource", json!({})).unwrap();
        crate::test_harness::frames(&mut h, 1);
        assert!(h.state().session.md_source.is_none(), "left source view");
        h.state_mut().run("text.insert", json!({"text": "Formatted edit. "})).unwrap();
        crate::test_harness::frames(&mut h, 1);
        h.state_mut().run("view.markdownSource", json!({})).unwrap();
        crate::test_harness::frames(&mut h, 1);
        crate::test_harness::frames(&mut h, 1);

        assert!(!enabled(&h.ctx, "edit.undo"), "undo must be reset on reopening pane");
        assert!(h.state_mut().run("edit.undo", json!({})).is_err(), "the engine refuses Undo while the pane is open");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

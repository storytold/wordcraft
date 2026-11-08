//! View › Markdown Source: the text pane that takes the page's place. The text itself and what it
//! means for the document are the engine's (`wordcraft_engine::cmd::md_source`).

use egui::text::{CCursor, CCursorRange};
use egui::{FontId, Key, KeyboardShortcut, Modifiers, Ui};
use serde_json::json;

use crate::WordApp;
use crate::clock::now_ms;
use crate::theme::Tokens;

/// How often a text with edits reaches the document while it is being typed, in milliseconds, so
/// crash recovery has them.
const APPLY_EVERY_MS: f64 = 5000.0;

/// Draw the pane in the place of the page.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // The page isn't drawn, so it can't have the keyboard.
    app.canvas.focused = false;
    app.canvas.want_focus = false;
    // A focused text box turns the global shortcuts off.
    if ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))) {
        let _ = app.run("file.save", json!({}));
    }
    let now = now_ms();
    if app.session.md_source.as_ref().is_some_and(|src| src.is_pending() && now - src.applied_at_ms > APPLY_EVERY_MS) {
        wordcraft_engine::cmd::md_source::apply(&mut app.session);
        if let Some(src) = app.session.md_source.as_mut() {
            src.applied_at_ms = now;
        }
    }
    let doc_id = app.session.document_id();
    let id = box_id(doc_id);
    let mut changed = false;
    let mut truncated = false;
    {
        let Some(src) = app.session.md_source.as_mut() else { return };
        if src.loses_content {
            egui::Frame::NONE.fill(t.hover).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(tl!(
                    "This document has content Markdown can't hold. Changing the text here removes it; one Undo afterwards brings it back."
                ));
            });
        }
        egui::ScrollArea::vertical().id_salt(id).auto_shrink(false).show(ui, |ui| {
            let size = ui.available_size();
            let mut out = egui::TextEdit::multiline(&mut src.text)
                .id(id)
                .hint_text(tl!("Markdown Source"))
                .font(FontId::monospace(14.0))
                .desired_width(f32::INFINITY)
                .min_size(size)
                .frame(egui::Frame::NONE.inner_margin(16))
                .text_color(t.text)
                .char_limit(wordcraft_engine::cmd::md_source::MAX_SOURCE_BYTES)
                .lock_focus(true)
                .show(ui);
            changed = out.response.changed();
            if changed && src.text.len() > wordcraft_engine::cmd::md_source::MAX_SOURCE_BYTES {
                let limit = src.text.floor_char_boundary(wordcraft_engine::cmd::md_source::MAX_SOURCE_BYTES);
                src.text.truncate(limit);
                src.caret = src.caret.min(limit);
                truncated = true;
            }
            let caret_changed = if src.place_caret {
                src.place_caret = false;
                let char_idx = src.text.get(..src.caret).map_or(0, |s| s.chars().count());
                let caret = CCursor::new(char_idx);
                out.state.cursor.set_char_range(Some(CCursorRange::one(caret)));
                out.state.store(ui.ctx(), id);
                out.response.request_focus();
                let spot = out.galley.pos_from_cursor(caret).translate(out.galley_pos.to_vec2());
                ui.scroll_to_rect(spot, Some(egui::Align::Center));
                true
            } else if let Some(range) = out.cursor_range {
                let old_caret = src.caret;
                let char_idx: usize = range.primary.index.into();
                src.caret = src.text.char_indices().nth(char_idx).map_or(src.text.len(), |(b, _)| b);
                old_caret != src.caret
            } else {
                false
            };
            if changed || caret_changed {
                invalidate_undo_cache(ui.ctx(), doc_id);
            }
        });
    }
    if truncated {
        app.status(tl!("Markdown Source takes up to 2 MB of text"));
    }
    if changed {
        app.session.dirty = true;
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(APPLY_EVERY_MS / 1000.0));
    }
}

/// The text box of a document's pane, found the same way by Undo and Redo as by the pane.
fn box_id(document: u64) -> egui::Id {
    egui::Id::unique("md_source").with(document)
}

/// The context storage key for caching the pane's undo/redo availability.
fn undo_cache_id(document: u64) -> egui::Id {
    egui::Id::unique("md_source_undo_cache").with(document)
}

/// Cached undo and redo capabilities for a document's Markdown Source text box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CachedUndoState {
    can_undo: bool,
    can_redo: bool,
}

/// Invalidates the cached undo and redo capability for `document`.
pub(crate) fn invalidate_undo_cache(ctx: &egui::Context, document: u64) {
    ctx.data_mut(|d| d.remove::<CachedUndoState>(undo_cache_id(document)));
}

/// A text box's state for its typing history: the selection and the text.
type Typed = (CCursorRange, String);

/// The text box's typing history, with the state Undo and Redo start from.
fn history(app: &WordApp) -> Option<(egui::Context, egui::text_edit::TextEditState, egui::util::undoer::Undoer<Typed>, Typed)> {
    let ctx = app.ctx.clone()?;
    let src = app.session.md_source.as_ref()?;
    let state = egui::text_edit::TextEditState::load(&ctx, box_id(app.session.document_id()))?;
    let undoer = state.undoer();
    let caret_chars = src.text[..src.text.floor_char_boundary(src.caret)].chars().count();
    let range = state.cursor.char_range().unwrap_or(CCursorRange::one(CCursor::new(caret_chars)));
    Some((ctx, state, undoer, (range, src.text.clone())))
}

/// Whether the pane's Undo (or Redo, with `redo`) has a step to take.
pub fn can_undo(app: &WordApp, redo: bool) -> bool {
    let Some(ctx) = app.ctx.as_ref() else { return false };
    let doc_id = app.session.document_id();
    let cache_key = undo_cache_id(doc_id);
    if let Some(cached) = ctx.data(|d| d.get_temp::<CachedUndoState>(cache_key)) {
        return if redo { cached.can_redo } else { cached.can_undo };
    }
    let Some((ctx, _, undoer, now)) = history(app) else { return false };
    let can_u = undoer.has_undo(&now);
    let can_r = undoer.has_redo(&now);
    ctx.data_mut(|d| d.insert_temp(cache_key, CachedUndoState { can_undo: can_u, can_redo: can_r }));
    if redo { can_r } else { can_u }
}

/// Undo (or Redo, with `redo`) typing in the pane, as Ctrl+Z (Ctrl+Y) does in it: the toolbar's
/// arrows and the Undo and Redo commands work on the text while it is open, not on the document.
pub fn undo(app: &mut WordApp, redo: bool) -> Result<serde_json::Value, String> {
    let Some((ctx, mut state, mut undoer, now)) = history(app) else { return Err(tl!("Nothing to undo").into()) };
    let step = if redo { undoer.redo(&now) } else { undoer.undo(&now) }.cloned();
    let Some((range, text)) = step else {
        return Err(if redo { tl!("Nothing to redo") } else { tl!("Nothing to undo") }.into());
    };
    state.cursor.set_char_range(Some(range));
    state.set_undoer(undoer);
    state.store(&ctx, box_id(app.session.document_id()));
    if let Some(src) = app.session.md_source.as_mut() {
        src.text = text;
        let char_idx: usize = range.primary.index.into();
        src.caret = src.text.char_indices().nth(char_idx).map_or(src.text.len(), |(b, _)| b);
    }
    invalidate_undo_cache(&ctx, app.session.document_id());
    app.session.dirty = true;
    ctx.request_repaint();
    Ok(json!({"text": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        )
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

    #[test]
    fn the_pane_shows_the_text_and_takes_the_keyboard() {
        let (mut a, dir) = opened("keys");
        a.run("view.markdownSource", json!({})).unwrap();
        let mut h = harness(a);
        h.run();
        assert!(!h.state().canvas.focused, "the page isn't drawn");
        assert!(h.ctx.text_edit_focused(), "the text box has the keyboard");
        assert!(h.get_all_by_value("# Heading\n\nSome text.\n").next().is_some(), "the Markdown is shown");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn typing_is_an_unsaved_change_and_ctrl_s_saves_it() {
        let (mut a, dir) = opened("save");
        a.run("view.markdownSource", json!({})).unwrap();
        let mut h = harness(a);
        h.run();
        h.state_mut().session.md_source.as_mut().unwrap().text.insert_str(0, "Typed. ");
        h.state_mut().session.dirty = true;
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
        h.run();
        let saved = std::fs::read_to_string(dir.join("note.md")).unwrap();
        assert!(saved.contains("Typed."), "{saved}");
        assert!(!h.state().session.dirty);
        assert!(h.state().session.md_source.is_some(), "saving keeps the pane");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_arrows_undo_and_redo_typing_in_the_pane() {
        let (mut a, dir) = opened("arrows");
        a.run("view.markdownSource", json!({})).unwrap();
        let mut h = harness(a);
        h.run();
        assert!(!crate::chrome::qat_enabled(h.state(), "edit.undo"), "nothing typed yet");
        h.event(egui::Event::Text("Typed. ".into()));
        h.run();
        let typed = h.state().session.md_source.as_ref().unwrap().text.clone();
        assert!(typed.contains("Typed. "), "{typed}");
        assert!(crate::chrome::qat_enabled(h.state(), "edit.undo"));
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run();
        assert_eq!(h.state().session.md_source.as_ref().unwrap().text, "# Heading\n\nSome text.\n", "Undo takes the typing back");
        assert!(h.state().session.md_source.is_some(), "and stays in the pane");
        assert!(crate::chrome::qat_enabled(h.state(), "edit.redo"));
        h.state_mut().run("edit.redo", json!({})).unwrap();
        h.run();
        assert_eq!(h.state().session.md_source.as_ref().unwrap().text, typed, "Redo puts it back");
        assert!(h.state_mut().run("edit.redo", json!({})).is_err(), "nothing more to redo");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_rewrite_warning_stays_longer_than_other_messages() {
        let (mut a, dir) = opened("warning");
        a.run("text.insert", json!({"text": "Formatted edit. "})).unwrap();
        a.run("file.save", json!({})).unwrap();
        let (msg, at) = a.status_msg.clone().unwrap();
        assert!(msg.contains("writing the Markdown out again"), "{msg}");
        let left = at + crate::app::STATUS_MS - now_ms();
        assert!(left > 15_000.0, "shown for about 20 seconds, {left} ms left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_ribbon_greys_out_edits_while_the_pane_is_open() {
        let (mut a, dir) = opened("grey");
        assert_eq!(a.session.why_disabled("format.bold"), None);
        a.run("view.markdownSource", json!({})).unwrap();
        assert_eq!(a.session.why_disabled("format.bold"), Some("leave Markdown Source first"));
        assert_eq!(a.session.why_disabled("view.markdownSource"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Verifies that typing beyond `MAX_SOURCE_BYTES` truncates at boundary and warns.
    #[test]
    fn text_exceeding_max_source_bytes_is_truncated_and_warns() {
        let (mut a, dir) = opened("oversize");
        a.run("view.markdownSource", json!({})).unwrap();
        let limit = wordcraft_engine::cmd::md_source::MAX_SOURCE_BYTES;
        let half = limit / 2;
        a.session.md_source.as_mut().unwrap().text = "é".repeat(half);
        let mut h = harness(a);
        h.run();
        h.event(egui::Event::Text("ééééé".into()));
        h.run();
        let text_len = h.state().session.md_source.as_ref().unwrap().text.len();
        assert_eq!(text_len, limit);
        let status = h.state().status_msg.as_ref().map(|(s, _)| s.as_str());
        assert_eq!(status, Some("Markdown Source takes up to 2 MB of text"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

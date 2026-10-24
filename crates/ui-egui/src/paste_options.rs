//! A small, Word-style Paste Options button beside the end of the latest paste.
//!
//! It follows the pasted text while scrolling/zooming. Switching styles replays **only**
//! the immediately preceding paste, using the source captured at insertion time.

use egui::{Order, Pos2, vec2};
use serde_json::{Value, json};
use wordcraft_doc::Pos;

use crate::WordApp;

/// The three text pasting choices Word offers after insertion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMode {
    Source,
    Merge,
    Text,
}

impl PasteMode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Merge => "merge",
            Self::Text => "text",
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Source => "edit.paste",
            Self::Merge => "edit.pasteMerge",
            Self::Text => "edit.pasteText",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "merge" => Self::Merge,
            "text" => Self::Text,
            _ => Self::Source,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Source => "Paste",
            Self::Merge => "Merge Formatting",
            Self::Text => "Keep Text Only",
        }
    }
}

/// Last successful paste, captured only when it produced an undoable edit.
/// The document, caret and revision guard against replacing unrelated later edits.
pub struct PostPaste {
    pub params: Value,
    pub mode: PasteMode,
    document: u64,
    caret: Pos,
    revision: u64,
    undo_steps: u64,
}

impl PostPaste {
    /// Remember a paste once the engine finished applying it.
    pub fn capture(app: &WordApp, mut params: Value, mode: PasteMode, undo_before: u64) -> Option<Self> {
        if app.session.undo_steps_so_far() <= undo_before {
            return None;
        }
        // In-app rich fragments may not supply system text. Store the clipboard's exact
        // text representation so switching back to source can recognize that fragment.
        if params.get("text").is_none()
            && let Some(fragment) = app.session.clipboard.as_ref()
            && let Some(obj) = params.as_object_mut()
        {
            obj.insert("text".into(), Value::String(fragment.clipboard_text()));
        }
        Some(Self {
            params,
            mode,
            document: app.session.document_id(),
            caret: app.session.sel.focus.clone(),
            revision: app.session.rev(),
            undo_steps: app.session.undo_steps_so_far(),
        })
    }

    /// A paste is replaceable only while it remains the most recent, untouched edit.
    pub fn current(&self, app: &WordApp) -> bool {
        self.document == app.session.document_id()
            && self.undo_steps == app.session.undo_steps_so_far()
            && self.revision == app.session.rev()
            && self.caret == app.session.sel.focus
    }
}

/// An ordinary paste uses the preference from File > Options > Advanced.
/// Picture pastes bypass this path in WordApp::run so they are unaffected.
pub fn default_command(pref: &str, with_text: bool) -> &'static str {
    if with_text { PasteMode::from_key(pref).command() } else { "edit.paste" }
}

/// Switch the last paste's formatting by undoing it and pasting the same captured source once.
/// An unrelated edit, selection movement or another open document cancels the operation.
pub fn switch(app: &mut WordApp, mode: PasteMode) {
    let Some(saved) = app.paste_options.take() else { return };
    if !saved.current(app) {
        return;
    }
    if mode == saved.mode {
        app.paste_options = Some(saved);
        return;
    }
    // Take the original paste off, restoring its pre-paste selection and caret. Do not read
    // the live system clipboard: it may have changed while the menu was open.
    if app.execute_user("edit.undo", json!({})).is_err() {
        return;
    }
    let before = app.session.undo_steps_so_far();
    if let Err(err) = app.execute_user(mode.command(), saved.params.clone()) {
        // The original paste remains available as Redo when a replacement fails.
        let _ = app.execute_user("edit.redo", json!({}));
        app.status(err);
        return;
    }
    app.paste_options = PostPaste::capture(app, saved.params, mode, before);
    app.canvas.want_focus = true;
}

/// Put the button just **below and to the right of the pasted text's final caret**, as Word
/// does—not beside the mouse, the ribbon, or a fixed viewport corner.
fn at_caret(caret: Pos2, height: f32, viewport: egui::Rect) -> Pos2 {
    let x = (caret.x + 8.0).clamp(viewport.min.x + 3.0, (viewport.max.x - 82.0).max(viewport.min.x + 3.0));
    let y = (caret.y + height + 3.0).clamp(viewport.min.y + 3.0, (viewport.max.y - 28.0).max(viewport.min.y + 3.0));
    Pos2::new(x, y)
}

/// Paint one accessible, Word-like icon tile with the clipboard pictogram and a small
/// formatting marker. Unlike the ribbon's text menu, the three options fit side by side.
fn paste_mode_tile(ui: &mut egui::Ui, mode: PasteMode, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(48.0, 40.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(mode.label())));
    let t = crate::theme::Tokens::get(ui.ctx());
    let bg = if selected {
        t.checked
    } else if response.hovered() {
        t.hover
    } else {
        t.balloon
    };
    ui.painter().rect_filled(rect.shrink(1.0), 4.0, bg);
    if selected || response.hovered() {
        ui.painter().rect_stroke(rect.shrink(1.0), 4.0, egui::Stroke::new(1.0, if selected { t.accent } else { t.border }), egui::StrokeKind::Inside);
    }
    crate::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center() - vec2(5.0, 0.0), vec2(25.0, 25.0)), "paste", t.icon, t.accent);
    ui.painter().text(
        rect.center() + vec2(14.0, 10.0),
        egui::Align2::CENTER_CENTER,
        match mode {
            PasteMode::Source => "S",
            PasteMode::Merge => "M",
            PasteMode::Text => "A",
        },
        egui::FontId::proportional(11.0),
        t.text,
    );
    response.on_hover_text(tl!(mode.label()))
}

/// Paint the paste button and its three paste modes in a foreground pop-up over the document.
/// Moving the caret, editing again, closing the document or opening another dialog removes it.
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    let Some(state) = app.paste_options.as_ref() else { return };
    if !state.current(app) || app.ui.backstage || app.dialog.is_some() {
        app.paste_options = None;
        return;
    }
    let Some((caret, height)) = crate::canvas::caret_screen(app) else { return };
    let viewport = app.canvas.viewport.unwrap_or_else(|| ctx.content_rect());
    if !viewport.contains(caret) {
        return; // Keep its document anchor while scrolled away.
    }
    let at = at_caret(caret, height, viewport.intersect(ctx.content_rect()));
    egui::Area::new(egui::Id::new("post_paste_options")).order(Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            let shortcut = if ctx.os() == egui::os::OperatingSystem::Mac { "⌘" } else { "Ctrl" };
            // Clipboard symbol and shortcut beside the end of the newly pasted text, as in Word.
            let menu = ui.menu_button(format!("     ({shortcut}) ▾"), |ui| {
                ui.label(egui::RichText::new(tl!("Paste as:")).strong());
                ui.horizontal(|ui| {
                    for mode in [PasteMode::Source, PasteMode::Merge, PasteMode::Text] {
                        let selected = app.paste_options.as_ref().is_some_and(|p| p.mode == mode);
                        let resp = paste_mode_tile(ui, mode, selected);
                        if resp.clicked() {
                            switch(app, mode);
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if ui.button(tl!("Set as Default")).clicked() {
                    app.paste_options = None;
                    let _ = app.run("file.options", json!({"pane": "advanced"}));
                    ui.close();
                }
            });
            menu.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Paste")));
            let t = crate::theme::Tokens::get(ui.ctx());
            crate::icons::paint(
                ui.painter(),
                egui::Rect::from_center_size(menu.response.rect.left_center() + vec2(12.0, 0.0), vec2(17.0, 17.0)),
                "paste",
                t.icon,
                t.accent,
            );
        });
    });
}

/// Match Word's position: immediately underneath and near the text end, clamped to the page area.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_is_aligned_to_paste_not_a_fixed_corner() {
        let view = egui::Rect::from_min_max(Pos2::new(10.0, 100.0), Pos2::new(500.0, 600.0));
        assert_eq!(at_caret(Pos2::new(200.0, 240.0), 18.0, view), Pos2::new(208.0, 261.0));
        assert_eq!(at_caret(Pos2::new(490.0, 590.0), 18.0, view), Pos2::new(418.0, 572.0));
    }

    /// Switching the pop-up style edits the already inserted text, not a second paste.
    #[test]
    fn a_paste_can_be_reformatted_without_duplicating_text() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Before ")), crate::Services::default());
        app.run("caret.docEnd", json!({})).unwrap();
        app.run("edit.paste", json!({"text": "after"})).unwrap();
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Before after");
        assert!(app.paste_options.as_ref().is_some_and(|p| p.current(&app)));
        switch(&mut app, PasteMode::Text);
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Before after");
        assert!(app.paste_options.as_ref().is_some_and(|p| p.mode == PasteMode::Text));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Before ");
        assert!(app.paste_options.is_none());
    }

    /// Replaying a tracked paste with another format must keep both ends of the move linked.
    #[test]
    fn changing_paste_format_preserves_tracked_move() {
        for mode in [PasteMode::Merge, PasteMode::Text] {
            let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            app.run("document.setText", json!({"text": "First one.\\nSecond one.\\nThird one."})).unwrap();
            app.run("file.setAuthor", json!({"name": "Ana"})).unwrap();
            app.run("review.trackChanges", json!({"value": true})).unwrap();
            app.run("select.range", json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}})).unwrap();
            app.run("edit.cut", json!({})).unwrap();
            app.run("caret.set", json!({"pos": {"block": 0, "off": 0}})).unwrap();
            app.run("edit.paste", json!({})).unwrap();
            assert!(app.paste_options.as_ref().is_some_and(|p| p.current(&app)), "original paste should offer a formatting choice");
            switch(&mut app, mode);
            let dest = app.session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap();
            let move_name = app.session.doc.move_name(dest.runs[0].props.ins);
            assert_eq!(move_name, Some("move1"), "{mode:?}: switching formatting must preserve the tracked move");
            assert_eq!(app.session.doc.revisions.iter().filter(|r| r.move_name.as_deref() == Some("move1")).count(), 2);
            assert!(app.paste_options.as_ref().is_some_and(|p| p.mode == mode));
        }
    }

    /// The contextual menu never undoes typing, navigation or a new paste after the first one.
    #[test]
    fn another_edit_dismisses_paste_options() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("edit.paste", json!({"text": "pasted"})).unwrap();
        assert!(app.paste_options.is_some());
        app.run("text.insert", json!({"text": "!"})).unwrap();
        assert!(app.paste_options.is_none());
        switch(&mut app, PasteMode::Merge);
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Pasted!");
    }

    /// The Advanced setting applies to the normal Paste shortcut without changing clipboard text.
    #[test]
    fn saved_paste_mode_is_used_for_next_text_paste() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.ui.default_paste = "merge".into();
        app.run("edit.paste", json!({"text": "hello"})).unwrap();
        assert!(app.paste_options.as_ref().is_some_and(|p| p.mode == PasteMode::Merge));
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "hello");
        let restored = crate::UiState::from_json(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        assert_eq!(restored.default_paste, "merge");
    }

    /// The option is available as a command, including without an open document.
    #[test]
    fn default_paste_setting_uses_a_validated_ui_command() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        assert!(crate::works_without_document("ui.defaultPaste"));
        assert_eq!(app.run("ui.defaultPaste", json!({})).unwrap(), json!({"defaultPaste": "source"}));
        assert_eq!(app.run("ui.defaultPaste", json!({"value": "merge"})).unwrap(), json!({"defaultPaste": "merge"}));
        assert!(app.run("ui.defaultPaste", json!({"value": "unknown"})).is_err());
        assert!(app.run("ui.defaultPaste", json!({"value": 123})).is_err());
        assert_eq!(app.run("ui.defaultPaste", json!({})).unwrap(), json!({"defaultPaste": "merge"}));
    }

    /// Wingdings-style symbol text differs from its clipboard representation. Save the latter,
    /// so changing paste style can still identify the original rich fragment.
    #[test]
    fn captured_rich_paste_uses_clipboard_text_for_symbol_fonts() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("document.setText", json!({"text": "ok J\u{F0FC}"})).unwrap();
        app.session.doc.format_range(&Pos::body(0, 3), &Pos::body(0, 7), &|c| c.font = Some("Wingdings".into())).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let fragment = app.session.clipboard.as_ref().unwrap();
        let clipboard_text = fragment.clipboard_text();
        assert_ne!(fragment.plain_text(), clipboard_text);
        app.run("caret.docEnd", json!({})).unwrap();
        let before = app.session.undo_steps_so_far();
        app.run("edit.paste", json!({"text": clipboard_text})).unwrap();
        let captured = PostPaste::capture(&app, json!({}), PasteMode::Source, before).unwrap();
        assert_eq!(captured.params["text"], clipboard_text);
    }

    /// History is capped at 500 entries: inserting another paste still shows Paste Options.
    #[test]
    fn paste_options_survive_a_full_undo_history() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        for _ in 0..500 {
            app.session.push_undo("earlier edit", app.session.doc.clone(), app.session.sel.clone());
        }
        let before = app.session.undo_steps_so_far();
        assert_eq!(app.session.undo_depth(), 500);
        app.run("edit.paste", json!({"text": "new"})).unwrap();
        assert_eq!(app.session.undo_depth(), 500);
        assert_eq!(app.session.undo_steps_so_far(), before + 1);
        assert!(app.paste_options.as_ref().is_some_and(|p| p.current(&app)));
    }

    #[test]
    fn paste_modes_keep_the_existing_engine_commands() {
        assert_eq!(PasteMode::Source.command(), "edit.paste");
        assert_eq!(PasteMode::Merge.command(), "edit.pasteMerge");
        assert_eq!(PasteMode::Text.command(), "edit.pasteText");
        assert_eq!(default_command("merge", true), "edit.pasteMerge");
        assert_eq!(default_command("text", false), "edit.paste");
    }
}

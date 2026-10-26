//! A persistent proofing form over the shared review commands, independent of language data.

use egui::Ui;
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_engine::Selection;

use crate::WordApp;

#[derive(Clone, Debug, Serialize)]
pub struct SpellingForm {
    issue: Value,
    replacement: String,
    message: String,
    #[serde(skip)]
    document: u64,
    #[serde(skip)]
    epoch: u64,
    #[serde(skip)]
    selection: Selection,
}

impl SpellingForm {
    pub fn new(app: &WordApp, issue: Value) -> Self {
        let replacement = issue.get("suggestions").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).unwrap_or("").into();
        Self {
            issue,
            replacement,
            message: String::new(),
            document: app.session.document_id(),
            epoch: app.session.ime_body_epoch(),
            selection: app.session.sel.clone(),
        }
    }

    fn done(&self) -> bool {
        self.issue.get("done").and_then(Value::as_bool) == Some(true)
    }

    // Other windows and commands can change a target while this form is open.
    // Never apply a cached suggestion to a different selection or document body.
    fn current(&self, app: &mut WordApp) -> bool {
        self.document == app.session.document_id()
            && self.epoch == app.session.ime_body_epoch()
            && self.selection == app.session.sel
            && app.session.run("review.suggestions", &json!({})).ok().as_ref() == Some(&self.issue)
    }

    fn next(&mut self, app: &mut WordApp, wrap: bool) {
        match app.execute("review.spelling", json!({"wrap": wrap})) {
            Ok(issue) => *self = Self::new(app, issue),
            Err(e) => self.message = e,
        }
    }

    fn act(&mut self, app: &mut WordApp, command: Option<&str>) {
        if !self.current(app) {
            self.message = tl!("The document or selection changed. Check again to continue.").into();
            return;
        }
        if let Some(command) = command {
            let params = if command == "review.applySuggestion" { json!({"text": self.replacement}) } else { json!({}) };
            if let Err(e) = app.run(command, params) {
                self.message = e;
                return;
            }
        }
        self.next(app, false);
    }
}

/// Changing or ignoring advances within the same form; Close and Escape dismiss it.
pub fn body(app: &mut WordApp, ui: &mut Ui, form: &mut SpellingForm) -> bool {
    ui.set_width(420.0);
    if form.done() {
        ui.label(tl!("The spelling and grammar check is complete."));
    } else {
        let spelling = form.issue.get("kind").and_then(Value::as_str) == Some("spelling");
        ui.horizontal_wrapped(|ui| {
            ui.label(tl!(if spelling { "Spelling" } else { "Grammar" }));
            ui.label(form.issue.get("lang").and_then(Value::as_str).unwrap_or(""));
        });
        ui.label(egui::RichText::new(form.issue.get("text").and_then(Value::as_str).unwrap_or("")).strong());
        ui.label(tl!(form.issue.get("message").and_then(Value::as_str).unwrap_or("")));
        ui.add_space(8.0);
        ui.label(tl!("Suggestions"));
        let suggestions: Vec<&str> = form.issue.get("suggestions").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        egui::ScrollArea::vertical().id_salt("spelling_suggestions").max_height(120.0).show(ui, |ui| {
            if suggestions.is_empty() {
                ui.label(egui::RichText::new(tl!("(no suggestions)")).italics());
            }
            for suggestion in &suggestions {
                if ui.selectable_label(form.replacement == *suggestion, *suggestion).clicked() {
                    form.replacement = (*suggestion).into();
                }
            }
        });
        let label = ui.label(tl!("Change to:"));
        ui.add(egui::TextEdit::singleline(&mut form.replacement).char_limit(1024).desired_width(400.0)).labelled_by(label.id);
        let can_change = (!form.replacement.is_empty() || suggestions.contains(&"")) && crate::widgets::enabled(app, "review.applySuggestion");
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(can_change, egui::Button::new(tl!("Change"))).clicked() {
                action = Some(Some("review.applySuggestion"));
            }
            if ui.button(tl!("Ignore Once")).clicked() {
                action = Some(None);
            }
            if spelling {
                if ui.button(tl!("Ignore All")).clicked() {
                    action = Some(Some("review.ignoreAll"));
                }
                if ui.button(tl!("Add to Dictionary")).clicked() {
                    action = Some(Some("review.addToDictionary"));
                }
            }
        });
        if let Some(command) = action {
            form.act(app, command);
        }
    }
    if !form.message.is_empty() {
        ui.colored_label(ui.visuals().error_fg_color, &form.message);
    }
    ui.add_space(8.0);
    let mut close = false;
    ui.horizontal(|ui| {
        if ui.button(tl!("Check Again")).clicked() {
            form.next(app, true);
        }
        close = ui.button(tl!("Close")).clicked();
    });
    close || crate::dialogs::cancel_key_pressed(ui)
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::{Document, Pos, StoryRef};
    use wordcraft_engine::Session;

    use super::*;
    use crate::{Services, dialogs::Dialog, dialogs_review::ReviewDialog};

    fn app(text: &str) -> WordApp {
        WordApp::new(Session::new(Document::from_text(text)), Services::default())
    }

    fn frame(ui: &mut Ui, app: &mut WordApp) {
        let ctx = ui.ctx().clone();
        app.logic(&ctx);
        app.ui(ui);
    }

    fn harness(text: &str) -> egui_kittest::Harness<'static, WordApp> {
        let mut app = app(text);
        app.run("review.spelling", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(frame, app);
        h.run_steps(6);
        h
    }

    fn form(app: &WordApp) -> &SpellingForm {
        match &app.dialog {
            Some(Dialog::Review(d)) => match &**d {
                ReviewDialog::Spelling(f) => f,
                _ => panic!("spelling form expected"),
            },
            _ => panic!("spelling dialog expected"),
        }
    }

    #[test]
    fn spelling_review_dialog_changes_a_suggestion_and_undo_restores_the_word() {
        let mut h = harness("The quick brwn fox.");
        assert!(!h.state().canvas.context_menu_open);
        h.get_by_label("brown").click();
        h.run_steps(2);
        h.get_by_label("Change").click();
        h.run_steps(3);
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "The quick brown fox.");
        assert!(form(h.state()).done());
        h.get_by_label("Close").click();
        h.run_steps(2);
        assert!(h.state().dialog.is_none());
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "The quick brwn fox.");
    }

    #[test]
    fn spelling_review_dialog_ignore_once_advances_and_finishes_without_changing_text() {
        let text = "The brwn fox and brwn dog.";
        let mut h = harness(text);
        let first = h.state().session.sel.clone();
        h.get_by_label("Ignore Once").click();
        h.run_steps(3);
        assert_ne!(h.state().session.sel, first);
        assert_eq!(form(h.state()).issue["text"], "brwn");
        h.get_by_label("Ignore Once").click();
        h.run_steps(3);
        assert!(form(h.state()).done());
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), text);
        h.get_by_label("Check Again").click();
        h.run_steps(3);
        assert_eq!(h.state().session.sel, first, "Ignore Once never adds a word to the dictionary");
    }

    #[test]
    fn spelling_review_dialog_ignore_all_and_add_to_dictionary_advance() {
        for (word, button) in [("zqproofingdialogignore", "Ignore All"), ("zqproofingdialogcustom", "Add to Dictionary")] {
            let text = format!("The {word} fox and {word} dog.");
            let mut h = harness(&text);
            h.get_by_label(button).click();
            h.run_steps(3);
            assert!(form(h.state()).done());
            assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), text);
            assert!(h.state_mut().session.run("review.issues", &json!({})).unwrap().as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn spelling_review_dialog_refuses_a_stale_selection_or_replaced_body() {
        for replace in [false, true] {
            let mut a = app("The brwn fox and brwn dog.");
            let issue = a.execute("review.spelling", json!({})).unwrap();
            assert!(a.dialog.is_none(), "programmatic review calls never open a window");
            let mut f = SpellingForm::new(&a, issue);
            f.replacement = "brown".into();
            if replace {
                a.execute("document.setText", json!({"text": "The brwn fox and brwn dog."})).unwrap();
            } else {
                a.session.sel = Selection::caret(Pos::body(0, 0));
            }
            let before = a.session.doc.plain_text(StoryRef::Body);
            f.act(&mut a, Some("review.applySuggestion"));
            assert!(!f.message.is_empty());
            assert_eq!(a.session.doc.plain_text(StoryRef::Body), before);
        }
    }

    #[test]
    fn spelling_review_dialog_shows_completion_without_issues_and_escape_closes() {
        let mut h = harness("The quick brown fox.");
        assert!(form(h.state()).done());
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().dialog.is_none());
    }

    #[test]
    #[ignore = "offscreen visual review; requires a software or hardware graphics adapter"]
    fn spelling_review_dialog_preview() {
        let mut a = app("The quick brwn fox.");
        a.run("review.spelling", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).wgpu().build_ui_state(frame, a);
        for (name, visuals) in [("light", egui::Visuals::light()), ("dark", egui::Visuals::dark())] {
            h.ctx.set_visuals(visuals);
            h.run_steps(6);
            h.render().unwrap().save(format!("spelling-review-{name}.png")).unwrap();
        }
    }
}

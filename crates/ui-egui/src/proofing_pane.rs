//! Language-aware proofing task pane, laid out from black-box observation of Word on Windows.

use egui::{Align, Align2, Sense, Stroke, Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_engine::Selection;

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};

#[derive(Clone, Debug, Serialize)]
pub struct SpellingForm {
    issue: Value,
    replacement: String,
    message: String,
    context_before: String,
    context_after: String,
    manual: bool,
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
        let (a, b) = app.session.sel.ordered();
        let para = app.session.doc.para_at(&a);
        let before = para.and_then(|p| p.text.get(..a.off)).unwrap_or("").rsplit(['.', '!', '?']).next().unwrap_or("").trim_start();
        let mut before: Vec<char> = before.chars().rev().take(90).collect();
        before.reverse();
        let after = para.and_then(|p| p.text.get(b.off..)).unwrap_or("").split_inclusive(['.', '!', '?']).next().unwrap_or("");
        Self {
            issue,
            replacement,
            message: String::new(),
            context_before: before.into_iter().collect(),
            context_after: after.chars().take(90).collect(),
            manual: false,
            document: app.session.document_id(),
            epoch: app.session.ime_body_epoch(),
            selection: app.session.sel.clone(),
        }
    }

    fn done(&self) -> bool {
        self.issue.get("done").and_then(Value::as_bool) == Some(true)
    }

    #[cfg(test)]
    pub(crate) fn content_text(&self) -> Vec<String> {
        let mut text: Vec<String> =
            self.issue.get("suggestions").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        text.push(format!("{}{}{}", self.context_before, self.issue.get("text").and_then(Value::as_str).unwrap_or(""), self.context_after));
        text
    }

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

/// The shared task-pane host supplies the title, menu, Close button, docking and keyboard routing.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let Some(mut form) = app.spelling_review.take() else { return };
    egui::Frame::NONE.inner_margin(egui::Margin { left: 4, right: 16, top: 0, bottom: 0 }).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| body(app, ui, &mut form));
    });
    app.spelling_review = Some(form);
}

fn action_link(ui: &mut Ui, text: &str) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(tl!(text)).color(Tokens::get(ui.ctx()).text).underline()).frame(false)).clicked()
}

fn context(ui: &mut Ui, app: &mut WordApp, form: &mut SpellingForm, spelling: bool) {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width();
    egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.input_border)).corner_radius(2.0).inner_margin(8).show(ui, |ui| {
        ui.set_min_width((width - 18.0).max(0.0));
        ui.horizontal(|ui| {
            let mut job = egui::text::LayoutJob::default();
            let format = egui::TextFormat { font_id: regular(12.0), color: t.text, ..Default::default() };
            job.append(&form.context_before, 0.0, format.clone());
            let marked = egui::TextFormat { underline: Stroke::new(1.0, if spelling { t.red } else { t.blue }), ..format.clone() };
            job.append(form.issue.get("text").and_then(Value::as_str).unwrap_or(""), 0.0, marked);
            job.append(&form.context_after, 0.0, format);
            let text_width = (ui.available_width() - 36.0).max(20.0);
            ui.allocate_ui_with_layout(vec2(text_width, 30.0), egui::Layout::top_down(Align::Min), |ui| {
                ui.set_min_width(text_width);
                ui.add(egui::Label::new(job).wrap());
            });
            let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Read Aloud")));
            crate::icons::paint(ui.painter(), rect.shrink(4.0), "readAloud", t.icon, t.accent);
            if response.on_hover_text(tl!("Read Aloud")).clicked() {
                if form.current(app) {
                    if let Err(e) = app.run("review.readAloud", json!({})) {
                        form.message = e;
                    }
                } else {
                    form.message = tl!("The document or selection changed. Check again to continue.").into();
                }
            }
        });
    });
}

fn body(app: &mut WordApp, ui: &mut Ui, form: &mut SpellingForm) {
    let t = Tokens::get(ui.ctx());
    if form.done() {
        ui.label(tl!("The spelling and grammar check is complete."));
        ui.add_space(8.0);
        if action_link(ui, "Check Again") {
            form.next(app, true);
        }
        return;
    }
    let spelling = form.issue.get("kind").and_then(Value::as_str) == Some("spelling");
    ui.label(egui::RichText::new(tl!(if spelling { "Spelling" } else { "Grammar" })).font(regular(14.0)))
        .on_hover_text(form.issue.get("lang").and_then(Value::as_str).unwrap_or(""));
    let message = if spelling { tl!("Not in Dictionary") } else { tl!(form.issue.get("message").and_then(Value::as_str).unwrap_or("")) };
    ui.label(egui::RichText::new(message).font(regular(11.0)).color(t.text_dim));
    ui.add_space(8.0);
    context(ui, app, form, spelling);
    ui.add_space(12.0);
    ui.label(egui::RichText::new(tl!("Suggestions")).font(regular(11.0)).color(t.text_dim));
    let suggestions: Vec<String> =
        form.issue.get("suggestions").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let mut change = None;
    if suggestions.is_empty() {
        ui.label(egui::RichText::new(tl!("(no suggestions)")).italics());
    } else {
        egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.input_border)).corner_radius(2.0).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().id_salt("proofing_suggestions").max_height(210.0).show(ui, |ui| {
                for (index, suggestion) in suggestions.iter().enumerate() {
                    ui.push_id(index, |ui| {
                        if index > 0 {
                            ui.separator();
                        }
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 34.0).max(20.0);
                            let (rect, response) = ui.allocate_exact_size(vec2(width, 64.0), Sense::click());
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, suggestion));
                            if response.hovered() {
                                ui.painter().rect_filled(rect, 0.0, t.hover);
                            }
                            ui.painter().with_clip_rect(rect.shrink2(vec2(10.0, 0.0))).text(
                                rect.left_center() + vec2(12.0, 0.0),
                                Align2::LEFT_CENTER,
                                suggestion,
                                semibold(12.5),
                                t.text,
                            );
                            if response.clicked() {
                                change = Some(suggestion.clone());
                            }
                            ui.menu_button("⌄", |ui| {
                                if ui.button(tl!("Change…")).clicked() {
                                    form.replacement.clone_from(suggestion);
                                    form.manual = true;
                                    ui.close();
                                }
                                if spelling && ui.button(tl!("Add to AutoCorrect")).clicked() {
                                    let word = form.issue.get("text").and_then(Value::as_str).unwrap_or("");
                                    let params = json!({"add": {"from": word, "to": suggestion}});
                                    if form.current(app) {
                                        if let Err(e) = app.run("tools.autocorrect", params) {
                                            form.message = e;
                                        }
                                    } else {
                                        form.message = tl!("The document or selection changed. Check again to continue.").into();
                                    }
                                    ui.close();
                                }
                                if ui.button(tl!("AutoCorrect Options…")).clicked() {
                                    let _ = app.run("tools.autocorrect", json!({}));
                                    ui.close();
                                }
                            })
                            .response
                            .on_hover_text(tl!("More options"));
                        });
                    });
                }
            });
        });
    }
    if let Some(replacement) = change {
        form.replacement = replacement;
        form.act(app, Some("review.applySuggestion"));
        return;
    }
    ui.add_space(6.0);
    if action_link(ui, "Ignore Once") {
        form.act(app, None);
    } else if spelling && action_link(ui, "Ignore All") {
        form.act(app, Some("review.ignoreAll"));
    } else if spelling && action_link(ui, "Add to Dictionary") {
        form.act(app, Some("review.addToDictionary"));
    }
    if suggestions.is_empty() && action_link(ui, "Change…") {
        form.manual = true;
    }
    if form.manual {
        ui.add_space(8.0);
        let label = ui.label(tl!("Change to:"));
        ui.add(egui::TextEdit::singleline(&mut form.replacement).char_limit(1024)).labelled_by(label.id);
        if ui.button(tl!("Change")).clicked() {
            form.act(app, Some("review.applySuggestion"));
        }
    }
    if !form.message.is_empty() {
        ui.colored_label(ui.visuals().error_fg_color, &form.message);
        if action_link(ui, "Check Again") {
            form.next(app, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::{Document, Pos, StoryRef};
    use wordcraft_engine::Session;

    use super::*;
    use crate::Services;

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
        app.spelling_review.as_ref().expect("proofing pane expected")
    }

    #[test]
    fn spelling_review_dialog_changes_a_suggestion_and_undo_restores_the_word() {
        let mut h = harness("The quick brwn fox.");
        assert!(!h.state().canvas.context_menu_open);
        let replacement = form(h.state()).issue["suggestions"][0].as_str().unwrap().to_string();
        h.get_by_label(&replacement).click();
        h.run_steps(3);
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), format!("The quick {replacement} fox."));
        assert!(form(h.state()).done());
        h.get_by_label("Close").click();
        h.run_steps(2);
        assert!(h.state().spelling_review.is_none());
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
    fn spelling_review_dialog_selected_spelling_stays_actionable_when_grammar_overlaps() {
        let mut a = app("A sentense sentense.");
        a.session.sel = Selection::caret(Pos::body(0, 11));
        a.run("review.spelling", json!({"wrap": false})).unwrap();
        let mut f = a.spelling_review.take().unwrap();
        assert_eq!(f.issue["kind"], "spelling");
        assert!(f.current(&mut a), "the selected typo is current despite overlapping grammar");
        f.replacement = "sentence".into();
        f.act(&mut a, Some("review.applySuggestion"));
        assert!(f.message.is_empty());
        assert_eq!(a.session.doc.plain_text(StoryRef::Body), "A sentense sentence.");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(a.session.doc.plain_text(StoryRef::Body), "A sentense sentense.");
    }

    #[test]
    fn spelling_review_dialog_refuses_a_stale_selection_or_replaced_body() {
        for replace in [false, true] {
            let mut a = app("The brwn fox and brwn dog.");
            let issue = a.execute("review.spelling", json!({})).unwrap();
            assert!(a.dialog.is_none() && a.spelling_review.is_none(), "programmatic calls never open a pane or window");
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
    fn spelling_review_dialog_shows_completion_without_issues_and_close_returns_to_document() {
        let mut h = harness("The quick brown fox.");
        assert!(form(h.state()).done());
        h.get_by_label("Close").click();
        h.run_steps(2);
        assert!(h.state().spelling_review.is_none());
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

    #[test]
    fn spelling_review_dialog_pane_is_local_to_each_window_and_clears_on_document_switch() {
        let mut a = app("The brwn fox.");
        a.run("review.spelling", json!({})).unwrap();
        let mut other = crate::window_ui::WindowUi::new(a.session.document_id());
        a.swap_window(&mut other);
        assert!(a.spelling_review.is_none());
        a.swap_window(&mut other);
        assert_eq!(form(&a).issue["text"], "brwn");
        a.run("file.new", json!({})).unwrap();
        assert!(a.spelling_review.is_none());
    }
}

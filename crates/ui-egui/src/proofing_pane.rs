//! Language-aware proofing task pane, laid out from black-box observation of Word on Windows.

use craft::behavior::{DisplayOrder, ItemSource};
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{ActivateOn, Bind, CollectionEvent, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::menu::Menu;
use craft::ui::notices::Notice;
use craft::ui::style::{Span, TypeRole};
use craft::ui::text::TextField;
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
    context_before: String,
    context_after: String,
    manual: bool,
    #[serde(skip)]
    document: u64,
    #[serde(skip)]
    epoch: u64,
    #[serde(skip)]
    selection: Selection,
    /// The suggestion row whose more-options menu is open.
    #[serde(skip)]
    menu_row: Option<usize>,
}

/// The suggestions as the rows of a list, keyed by position.
struct Suggestions<'a>(&'a [String]);

impl DisplayOrder<usize> for Suggestions<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).map(|name| std::borrow::Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<usize> for Suggestions<'_> {}

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
            menu_row: None,
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

/// What the user asked for in one pass; it runs after the components are drawn, so the body never
/// holds the form while a component does.
enum Ask {
    CheckAgain,
    ReadAloud,
    Change(String),
    Manual(String),
    AddToAutoCorrect(String),
    AutoCorrectOptions,
    IgnoreOnce,
    IgnoreAll,
    AddToDictionary,
    ChangeTyped,
}

/// The more-options menu rows of a suggestion.
const MORE_CHANGE: u8 = 0;
const MORE_ADD: u8 = 1;
const MORE_OPTIONS: u8 = 2;

/// The shared task-pane host supplies the title, menu, Close button, docking and keyboard routing.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    // The form leaves the app while the body runs, so the body can call `app.run`.
    let Some(mut form) = app.spelling_review.take() else { return };
    Stack::new().show(ui, |ui, _| body(app, ui, &mut form));
    app.spelling_review = Some(form);
}

fn body(app: &mut WordApp, ui: &mut Ui, form: &mut SpellingForm) {
    if form.done() {
        Notice::new(Tone::Good, tl!("The spelling and grammar check is complete.")).show(ui);
        if Button::new(tl!("Check Again")).show(ui).clicked() {
            form.next(app, true);
        }
        return;
    }
    let spelling = form.issue.get("kind").and_then(Value::as_str) == Some("spelling");
    let lang = form.issue.get("lang").and_then(Value::as_str).unwrap_or("").to_string();
    let heading = Text::new(tl!(if spelling { "Spelling" } else { "Grammar" })).role(TypeRole::Heading);
    if lang.is_empty() {
        heading.show(ui);
    } else {
        heading.tooltip(lang).show(ui);
    }
    let message = if spelling { tl!("Not in Dictionary") } else { tl!(form.issue.get("message").and_then(Value::as_str).unwrap_or("")) };
    Text::new(message.to_owned()).shade(Shade::Muted).show(ui);

    let mut ask: Option<Ask> = None;
    let flagged = form.issue.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let spans = vec![
        Span::new(form.context_before.clone()),
        Span::new(flagged.clone()).flag(if spelling { Tone::Bad } else { Tone::Info }),
        Span::new(form.context_after.clone()),
    ];
    Row::new().show(ui, |ui, _| {
        Text::styled(spans.clone()).show(ui);
        // PLAN: assumed IconSpec::svg("readAloud") from WORDCRAFT_ICONS (rb.A7, ribbon-w13); the icon is registered by name
        if IconButton::new(crate::icons::spec("readAloud")).label(tl!("Read Aloud")).show(ui).clicked() {
            ask = Some(Ask::ReadAloud);
        }
    });

    Text::new(tl!("Suggestions")).shade(Shade::Muted).show(ui);
    let suggestions: Vec<String> =
        form.issue.get("suggestions").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    if suggestions.is_empty() {
        Text::new(tl!("(no suggestions)")).italic().show(ui);
    } else {
        // A click applies that suggestion; its more-options menu acts on that suggestion alone.
        let source = Suggestions(&suggestions);
        let listed = List::<usize>::new(tl!("Suggestions"))
            .row_menu()
            // One click applies a suggestion; a collection activates on a double-click or Enter unless told.
            .select(SelectMode::None)
            .activate(ActivateOn::Click)
            // The pane scrolls, so the list takes its few rows' full height instead of owning a second scroll.
            .natural_height(true)
            .show_items(ui, &source, &mut |key, _| ListRow::new(suggestions.get(*key).cloned().unwrap_or_default()));
        for event in &listed.events {
            match event {
                CollectionEvent::Activate(key) => {
                    if let Some(suggestion) = suggestions.get(*key) {
                        ask = Some(Ask::Change(suggestion.clone()));
                    }
                }
                CollectionEvent::ContextMenu(request) => form.menu_row = Some(request.key),
                _ => {}
            }
        }
        if let Some(row) = form.menu_row {
            let mut entries = vec![Entry::new(MORE_CHANGE, tl!("Change…"))];
            if spelling {
                entries.push(Entry::new(MORE_ADD, tl!("Add to AutoCorrect")));
            }
            entries.push(Entry::new(MORE_OPTIONS, tl!("AutoCorrect Options…")));
            let target = suggestions.get(row).cloned().unwrap_or_default();
            let menu = Menu::context(tl!("More options"), &listed.response).pick(ui, &entries);
            match menu.picked {
                Some(MORE_CHANGE) => ask = Some(Ask::Manual(target)),
                Some(MORE_ADD) => ask = Some(Ask::AddToAutoCorrect(target)),
                Some(MORE_OPTIONS) => ask = Some(Ask::AutoCorrectOptions),
                _ => {}
            }
            if menu.closed.is_some() {
                form.menu_row = None;
            }
        }
    }

    Row::new().show(ui, |ui, _| {
        if Button::new(tl!("Ignore Once")).show(ui).clicked() {
            ask = Some(Ask::IgnoreOnce);
        }
        if spelling && Button::new(tl!("Ignore All")).show(ui).clicked() {
            ask = Some(Ask::IgnoreAll);
        }
        if spelling && Button::new(tl!("Add to Dictionary")).show(ui).clicked() {
            ask = Some(Ask::AddToDictionary);
        }
        if suggestions.is_empty() && Button::new(tl!("Change…")).show(ui).clicked() {
            ask = Some(Ask::Manual(String::new()));
        }
    });

    if form.manual {
        let typed = TextField::new(tl!("Change to:"), Bind::given(form.replacement.clone())).limit(1024).show(ui);
        if let Some(text) = typed.events.committed() {
            form.replacement.clone_from(text);
        }
        if Button::new(tl!("Change")).show(ui).clicked() {
            ask = Some(Ask::ChangeTyped);
        }
    }
    if !form.message.is_empty() {
        Notice::new(Tone::Bad, form.message.clone()).show(ui);
        if Button::new(tl!("Check Again")).show(ui).clicked() {
            ask = Some(Ask::CheckAgain);
        }
    }

    let Some(ask) = ask else { return };
    match ask {
        Ask::CheckAgain => form.next(app, true),
        Ask::ReadAloud => {
            if form.current(app) {
                if let Err(e) = app.run("review.readAloud", json!({})) {
                    form.message = e;
                }
            } else {
                form.message = tl!("The document or selection changed. Check again to continue.").into();
            }
        }
        Ask::Change(suggestion) => {
            form.replacement = suggestion;
            form.act(app, Some("review.applySuggestion"));
        }
        Ask::Manual(suggestion) => {
            if !suggestion.is_empty() {
                form.replacement = suggestion;
            }
            form.manual = true;
        }
        Ask::AddToAutoCorrect(suggestion) => {
            let params = json!({"add": {"from": flagged, "to": suggestion}});
            if form.current(app) {
                if let Err(e) = app.run("tools.autocorrect", params) {
                    form.message = e;
                }
            } else {
                form.message = tl!("The document or selection changed. Check again to continue.").into();
            }
        }
        Ask::AutoCorrectOptions => {
            let _ = app.run("tools.autocorrect", json!({}));
        }
        Ask::IgnoreOnce => form.act(app, None),
        Ask::IgnoreAll => form.act(app, Some("review.ignoreAll")),
        Ask::AddToDictionary => form.act(app, Some("review.addToDictionary")),
        Ask::ChangeTyped => form.act(app, Some("review.applySuggestion")),
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
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(frame, app);
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
            assert!(a.dialog_host.is_empty() && a.spelling_review.is_none(), "programmatic calls never open a pane or window");
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
        // A notice is named by its text on more than one node (the notice and its message), so ask for any.
        let notice = egui_kittest::kittest::By::new().label("The spelling and grammar check is complete.");
        assert!(h.query_all(notice).next().is_some(), "the completion notice shows");
        h.get_by_label("Close").click();
        h.run_steps(2);
        assert!(h.state().spelling_review.is_none());
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

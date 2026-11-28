//! The Accessibility pane (the checker's findings, each a jump and maybe a fix) and the Alt Text
//! pane (the selected object's or table's alternative text).

use std::cell::Cell;
use std::rc::Rc;

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CollectionEvent, Show as _, ShowCollection as _, ShowContainer as _, WordSetCx as _};
use craft::ui::layout_components::Stack;
use craft::ui::list::{Finding, Findings, FindingsList, FindingsSummary, FindingsWords, Severity};
use craft::ui::notices::Notice;
use craft::ui::text::{AltText, AltTextField, TextField};
use egui::Ui;
use serde_json::{Value, json};
use wordcraft_doc::Pos;

use crate::WordApp;

/// One finding of the checker as a list finding; its id is its place in the checker's list.
fn finding(id: usize, issue: &Value) -> Finding {
    let severity = match issue["kind"].as_str() {
        Some("error") => Severity::Error,
        Some("warning") => Severity::Warning,
        _ => Severity::Info,
    };
    let finding = Finding::new(id as u64, severity, issue["issue"].as_str().unwrap_or(""));
    match issue["fix"].as_str() {
        Some(fix) => finding.detail(fix),
        None => finding,
    }
}

/// Findings the list shows before it scrolls the rest itself.
const FINDINGS_SHOWN: u16 = 12;

/// Review › Check Accessibility: the findings for the document as it is now.
pub fn accessibility(app: &mut WordApp, ui: &mut Ui) {
    // The findings are copied out: the body below calls `app` again.
    let issues: Vec<Value> = app.session.accessibility_findings().to_vec();
    if issues.is_empty() {
        Notice::new(Tone::Neutral, tl!("No accessibility issues found.")).show(ui);
        return;
    }
    let findings: Vec<Finding> = issues.iter().enumerate().map(|(i, v)| finding(i, v)).collect();
    let source = Findings::new(&findings);
    let acted: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let mut activated: Option<usize> = None;
    Stack::new().show(ui, |ui, _| {
        FindingsSummary::new(&findings).show(ui);
        // The pane body scrolls and a collection owns its own scroll: the list shows this many findings and scrolls the rest.
        let out = FindingsList::new(tl!("Accessibility findings")).max_rows(FINDINGS_SHOWN).show_items(ui, &source, &mut |key, icx| {
            let at = *key as usize;
            let words = icx.words::<FindingsWords>();
            let Some(found) = findings.get(at) else { return craft::ui::list::ListRow::new(String::new()) };
            let row = found.row(&words);
            let label = issues.get(at).and_then(|i| i.get("action")).filter(|a| a.is_object()).and_then(|a| a["label"].as_str());
            match label {
                Some(label) => {
                    let acted = Rc::clone(&acted);
                    row.below(Button::new(tl!(label).to_string()), move |b| {
                        if b.clicked() {
                            acted.set(Some(at));
                        }
                    })
                }
                None => row,
            }
        });
        for event in &out.events {
            if let CollectionEvent::Activate(key) = event {
                activated = Some(*key as usize);
            }
        }
    });
    let ctx = ui.ctx().clone();
    if let Some(at) = acted.get().filter(|at| issues.get(*at).is_some()) {
        let issue = &issues[at];
        go_to(app, &ctx, issue);
        if let Some(action) = issue.get("action").filter(|a| a.is_object())
            && let Some(cmd) = action["command"].as_str()
        {
            let _ = app.run(cmd, action.get("params").cloned().unwrap_or_else(|| json!({})));
        }
    } else if let Some(issue) = activated.and_then(|at| issues.get(at)) {
        go_to(app, &ctx, issue);
    }
}

/// Select the object an issue is about, or put the caret where it is, and give the document focus.
fn go_to(app: &mut WordApp, ctx: &egui::Context, issue: &Value) {
    if let Some(sel) = issue.get("select").filter(|s| s.is_object()) {
        let _ = app.run("select.range", sel.clone());
    } else if let Some(pos) = issue.get("pos").filter(|p| p.is_object()) {
        let _ = app.run("caret.set", json!({"pos": pos}));
    }
    app.canvas.focus_page(ctx);
}

/// Alt Text: the selected picture's, shape's, chart's or group's description and decorative mark,
/// or the title and description of the table the caret is in.
pub fn alt_text(app: &mut WordApp, ui: &mut Ui) {
    if let Some((pos, obj)) = wordcraft_engine::cmd::objects::selected(&app.session) {
        let current = AltText::new(obj.alt_text()).decorative(obj.is_decorative());
        let mut out = None;
        Stack::new().show(ui, |ui, _| {
            craft::ui::display::Text::new(tl!("Describe what this object shows, for people who can't see it.")).show(ui);
            out = AltTextField::new(tl!("Description:"), Bind::given(current.clone())).show(ui).events.committed().cloned();
            craft::ui::display::Text::new(tl!("Decorative objects only add visual interest: screen readers skip them.")).show(ui);
        });
        // An edit that lands after the selection moved belongs to no object.
        if let Some(new) = out
            && still_selected(app, &pos)
        {
            if new.text != current.text {
                let text = match &new.text {
                    craft::behavior::Mixed::Same(t) => t.clone(),
                    _ => String::new(),
                };
                let _ = app.run("object.altText", json!({"text": text}));
            }
            if new.decorative != current.decorative {
                let decorative = matches!(new.decorative, craft::behavior::Mixed::Same(true));
                let _ = app.run("object.altText", json!({"decorative": decorative}));
            }
        }
        return;
    }
    let focus = app.session.sel.focus.clone();
    let table = focus
        .path
        .cell()
        .and_then(|(tp, _, _)| app.session.doc.table(focus.story, &tp).map(|t| (t.props.caption.clone(), t.props.description.clone())));
    if let Some((caption, description)) = table {
        let (mut title, mut text) = (None, None);
        Stack::new().show(ui, |ui, _| {
            craft::ui::display::Text::new(tl!("Describe what this table shows, for people who can't see it.")).show(ui);
            let current = caption.clone().unwrap_or_default();
            title = TextField::new(tl!("Title:"), Bind::given(current)).show(ui).events.committed().cloned();
            let current = description.clone().unwrap_or_default();
            text = TextField::new(tl!("Description:"), Bind::given(current)).lines(5).show(ui).events.committed().cloned();
        });
        if let Some(t) = title.filter(|t| Some(t.as_str()) != caption.as_deref()) {
            let _ = app.run("table.altText", json!({"title": t}));
        }
        if let Some(t) = text.filter(|t| Some(t.as_str()) != description.as_deref()) {
            let _ = app.run("table.altText", json!({"text": t}));
        }
        return;
    }
    Notice::new(Tone::Neutral, tl!("Select a picture, shape, chart or table to describe it.")).show(ui);
}

fn still_selected(app: &WordApp, pos: &Pos) -> bool {
    wordcraft_engine::cmd::objects::selected(&app.session).is_some_and(|(p, _)| &p == pos)
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn frame(ui: &mut Ui, app: &mut WordApp) {
        let ctx = ui.ctx().clone();
        app.logic(&ctx);
        app.ui(ui);
    }

    fn harness(mut app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        app.session.view.alt_text_pane = true;
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(frame, app);
        h.run_steps(6);
        h
    }

    #[test]
    fn table_title_and_description_are_named_and_commit_independently() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.table", json!({"rows": 1, "cols": 1})).unwrap();
        let mut h = harness(app);
        h.get_by_role_and_label(Role::TextInput, "Title:").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, "Title:").type_text("Prices");
        h.run_steps(2);
        h.get_by_role_and_label(Role::MultilineTextInput, "Description:").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::MultilineTextInput, "Description:").type_text("Prices by year");
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, "Title:").click();
        h.run_steps(2);
        let app = h.state();
        let focus = &app.session.sel.focus;
        let (path, _, _) = focus.path.cell().unwrap();
        let table = app.session.doc.table(focus.story, &path).unwrap();
        assert_eq!(table.props.caption.as_deref(), Some("Prices"));
        assert_eq!(table.props.description.as_deref(), Some("Prices by year"));
    }

    #[test]
    fn object_description_commits_only_while_the_object_is_selected() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.shape", json!({"kind": "ellipse"})).unwrap();
        let (pos, _) = wordcraft_engine::cmd::objects::selected(&app.session).unwrap();
        assert!(still_selected(&app, &pos));
        let mut h = harness(app);
        h.get_by_role_and_label(Role::MultilineTextInput, "Description:").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::MultilineTextInput, "Description:").type_text("A green circle");
        h.run_steps(2);
        h.get_by_role_and_label(Role::CheckBox, "Mark as decorative").click();
        h.run_steps(2);
        let (_, object) = wordcraft_engine::cmd::objects::selected(&h.state().session).unwrap();
        assert_eq!(object.alt_text(), "A green circle");
        assert!(object.is_decorative(), "the decorative mark commits with the object selected");
    }

    #[test]
    fn object_description_is_dropped_when_the_selection_moved_before_it_landed() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        // Text before the shape: a caret at the start of an empty paragraph sits on the shape, which
        // the engine counts as selecting it, so the move off it needs somewhere else to go.
        app.run("text.insert", json!({"text": "Before the shape "})).unwrap();
        app.run("insert.shape", json!({"kind": "ellipse"})).unwrap();
        let (pos, _) = wordcraft_engine::cmd::objects::selected(&app.session).unwrap();
        let mut h = harness(app);
        h.get_by_role_and_label(Role::MultilineTextInput, "Description:").click();
        h.run_steps(2);
        let depth = h.state().session.undo_depth();
        h.get_by_role_and_label(Role::MultilineTextInput, "Description:").type_text("Stale");
        h.state_mut().run("caret.set", json!({"pos": wordcraft_doc::Pos::body(0, 0)})).unwrap();
        assert!(!still_selected(h.state(), &pos), "the caret moved off the object");
        h.run_steps(3);
        assert_eq!(h.state().session.undo_depth(), depth, "an edit that lands after the selection moved writes nothing");
    }
}

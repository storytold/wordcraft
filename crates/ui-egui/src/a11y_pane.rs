//! The Accessibility pane (the checker's findings, grouped, each a jump and maybe a fix) and the
//! Alt Text pane (the selected object's or table's alternative text).

use egui::Ui;
use serde_json::{Value, json};
use wordcraft_doc::Pos;

use crate::WordApp;
use crate::theme::{regular, semibold};

/// Review › Check Accessibility: the findings for the document as it is now.
pub fn accessibility(app: &mut WordApp, ui: &mut Ui) {
    if crate::panes::header(ui, "Accessibility") {
        let _ = app.run("view.accessibilityPane", json!({"value": false}));
        return;
    }
    let issues = issues(app, ui);
    if issues.is_empty() {
        ui.label(egui::RichText::new(tl!("No accessibility issues found.")).weak());
        return;
    }
    let mut picked: Option<(Value, Option<Value>)> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (kind, title) in [("error", "Errors ({n})"), ("warning", "Warnings ({n})"), ("tip", "Tips ({n})")] {
            let group: Vec<&Value> = issues.iter().filter(|i| i["kind"] == kind).collect();
            if group.is_empty() {
                continue;
            }
            let heading = crate::i18n::fmt(tl!(title), &[("n", &group.len().to_string())]);
            egui::CollapsingHeader::new(egui::RichText::new(heading).font(semibold(13.0))).id_salt(("a11y", kind)).default_open(true).show(
                ui,
                |ui| {
                    for (k, i) in group.into_iter().enumerate() {
                        ui.push_id(k, |ui| {
                            let text = i["issue"].as_str().unwrap_or("");
                            let r = ui.add(egui::Button::new(egui::RichText::new(text).font(regular(12.5))).frame(false).wrap());
                            if let Some(fix) = i["fix"].as_str() {
                                ui.label(egui::RichText::new(fix).small().weak());
                            }
                            if r.clicked() {
                                picked = Some((i.clone(), None));
                            }
                            if let Some(a) = i.get("action").filter(|a| a.is_object())
                                && let Some(label) = a["label"].as_str()
                                && ui.small_button(tl!(label)).clicked()
                            {
                                picked = Some((i.clone(), Some(a.clone())));
                            }
                            ui.add_space(4.0);
                        });
                    }
                },
            );
        }
    });
    if let Some((issue, action)) = picked {
        go_to(app, &issue);
        if let Some(a) = action
            && let Some(cmd) = a["command"].as_str()
        {
            let _ = app.run(cmd, a.get("params").cloned().unwrap_or_else(|| json!({})));
        }
    }
}

/// The checker's findings, kept until the document changes.
fn issues(app: &mut WordApp, ui: &Ui) -> Vec<Value> {
    let key = egui::Id::new("a11y_issues");
    let stamp = (app.session.document_id(), app.session.rev());
    if let Some((s, v)) = ui.data(|d| d.get_temp::<((u64, u64), Vec<Value>)>(key))
        && s == stamp
    {
        return v;
    }
    let v = wordcraft_engine::cmd::accessibility::check(&app.session.doc);
    ui.data_mut(|d| d.insert_temp(key, (stamp, v.clone())));
    v
}

/// Select the object an issue is about, or put the caret where it is.
fn go_to(app: &mut WordApp, issue: &Value) {
    if let Some(sel) = issue.get("select").filter(|s| s.is_object()) {
        let _ = app.run("select.range", sel.clone());
    } else if let Some(pos) = issue.get("pos").filter(|p| p.is_object()) {
        let _ = app.run("caret.set", json!({"pos": pos}));
    }
    app.canvas.want_focus = true;
}

/// Alt Text: the selected picture's, shape's, chart's or group's description and decorative mark,
/// or the title and description of the table the caret is in.
pub fn alt_text(app: &mut WordApp, ui: &mut Ui) {
    if crate::panes::header(ui, "Alt Text") {
        let _ = app.run("view.altTextPane", json!({"value": false}));
        return;
    }
    if let Some((pos, obj)) = wordcraft_engine::cmd::objects::selected(&app.session) {
        let decorative = obj.is_decorative();
        ui.label(egui::RichText::new(tl!("Describe what this object shows, for people who can't see it.")).weak());
        ui.add_space(4.0);
        if let Some(text) = draft_box(ui, ("alt_obj", &pos), obj.alt_text(), !decorative, true)
            && still_selected(app, &pos)
        {
            let _ = app.run("object.altText", json!({"text": text}));
        }
        ui.add_space(6.0);
        let mut d = decorative;
        if ui.checkbox(&mut d, tl!("Mark as decorative")).changed() {
            let _ = app.run("object.altText", json!({"decorative": d}));
        }
        ui.label(egui::RichText::new(tl!("Decorative objects only add visual interest: screen readers skip them.")).small().weak());
        return;
    }
    let focus = app.session.sel.focus.clone();
    let table = focus
        .path
        .cell()
        .and_then(|(tp, _, _)| app.session.doc.table(focus.story, &tp).map(|t| (tp, t.props.caption.clone(), t.props.description.clone())));
    if let Some((tp, caption, description)) = table {
        ui.label(egui::RichText::new(tl!("Describe what this table shows, for people who can't see it.")).weak());
        ui.add_space(4.0);
        ui.label(tl!("Title:"));
        if let Some(t) = draft_box(ui, ("alt_table_title", &tp), caption.as_deref().unwrap_or(""), true, false) {
            let _ = app.run("table.altText", json!({"title": t}));
        }
        ui.add_space(4.0);
        ui.label(tl!("Description:"));
        if let Some(t) = draft_box(ui, ("alt_table_text", &tp), description.as_deref().unwrap_or(""), true, true) {
            let _ = app.run("table.altText", json!({"text": t}));
        }
        return;
    }
    ui.label(egui::RichText::new(tl!("Select a picture, shape, chart or table to describe it.")).weak());
}

fn still_selected(app: &WordApp, pos: &Pos) -> bool {
    wordcraft_engine::cmd::objects::selected(&app.session).is_some_and(|(p, _)| &p == pos)
}

/// A text box editing `current`; the draft lives in egui memory while it has focus (frame-local
/// buffers lose keystrokes). Returns the new text when editing ends with a change.
fn draft_box(ui: &mut Ui, key: impl std::hash::Hash + std::fmt::Debug, current: &str, enabled: bool, multiline: bool) -> Option<String> {
    let key = egui::Id::new(key);
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| current.to_string());
    let edit = if multiline { egui::TextEdit::multiline(&mut text).desired_rows(5) } else { egui::TextEdit::singleline(&mut text) };
    let r = ui.add_enabled(enabled, edit.desired_width(f32::INFINITY));
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
        return (text != current).then_some(text);
    }
    if !r.has_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
    }
    None
}

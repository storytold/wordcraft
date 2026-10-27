use super::common::{mi, stack};
use super::language::mi_check;
use super::*;

pub(super) fn mailings(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Create", None, app, |ui, app| {
        big(ui, app, "envelope", "Envelopes", "mailings.envelopes", json!({}), false);
        big(ui, app, "labels", "Labels", "mailings.labels", json!({}), false);
    });
    group(ui, "Start Mail Merge", None, app, |ui, app| {
        // The kinds of merge document the engine makes, the current one checked.
        menu_button(ui, app, "mailMerge", Some("Start Mail\nMerge"), "Start Mail Merge", true, "mailings.start", |ui, app| {
            let kind = app.session.merge.kind.clone();
            for (label, k) in [
                ("Letters", "letters"),
                ("E-mail Messages", "emails"),
                ("Envelopes", "envelopes"),
                ("Labels", "labels"),
                ("Directory", "directory"),
                ("Normal Word Document", "normal"),
            ] {
                let current = kind == k || (k == "normal" && kind.is_empty());
                mi_check(ui, app, label, current, "mailings.start", json!({"kind": k}));
            }
        });
        // The command needs data, so the button offers the two ways to give it (#240).
        menu_button(ui, app, "recipients", Some("Select\nRecipients"), "Select Recipients", true, "mailings.recipients", |ui, app| {
            mi(ui, app, "Type a New List…", "ui.dialog", json!({"name": "newRecipientList"}));
            mi(ui, app, "Use an Existing List…", "ui.openRecipientList", json!({}));
        });
        big(ui, app, "editRecipients", "Edit\nRecipient List", "mailings.editRecipients", json!({}), false);
    });
    group(ui, "Write & Insert Fields", None, app, |ui, app| {
        big(ui, app, "highlightFields", "Highlight\nMerge Fields", "mailings.highlightFields", json!({}), false);
        big(ui, app, "addressBlock", "Address\nBlock", "mailings.addressBlock", json!({}), false);
        big(ui, app, "greetingLine", "Greeting\nLine", "mailings.greetingLine", json!({}), false);
        big(ui, app, "mergeField", "Insert Merge\nField", "mailings.insertField", json!({}), false);
        stack(ui, |ui| {
            // The merge rules the engine knows; If and Skip Record If ask for their condition.
            menu_button(ui, app, "rules", Some("Rules"), "Rules", false, "mailings.rules", |ui, app| {
                mi(ui, app, "If…Then…Else…", "mailings.rules", json!({"rule": "IF"}));
                mi(ui, app, "Merge Record #", "mailings.rules", json!({"rule": "MERGEREC"}));
                mi(ui, app, "Next Record", "mailings.rules", json!({"rule": "NEXT"}));
                mi(ui, app, "Skip Record If…", "mailings.rules", json!({"rule": "SKIPIF"}));
            });
            small(ui, app, "matchFields", Some("Match Fields"), "Match Fields", "mailings.matchFields", json!({}), false);
        });
    });
    group(ui, "Preview Results", None, app, |ui, app| {
        big(ui, app, "preview", "Preview\nResults", "mailings.preview", json!({}), false);
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                small(ui, app, "first", None, "First Record", "mailings.first", json!({}), false);
                small(ui, app, "previous", None, "Previous Record", "mailings.previous", json!({}), false);
                record_box(ui, app);
                small(ui, app, "next", None, "Next Record", "mailings.next", json!({}), false);
                small(ui, app, "last", None, "Last Record", "mailings.last", json!({}), false);
            });
            small(ui, app, "find", Some("Find Recipient"), "Find Recipient", "mailings.findRecipient", json!({}), false);
            small(ui, app, "checkErrors", Some("Check for Errors"), "Check for Errors", "mailings.checkErrors", json!({}), false);
        });
    });
    group(ui, "Finish", None, app, |ui, app| {
        menu_button(ui, app, "finish", Some("Finish &\nMerge"), "Finish & Merge", true, "mailings.finish", |ui, app| {
            mi(ui, app, "Edit Individual Documents…", "mailings.finish", json!({}));
            mi(ui, app, "Print Documents…", "mailings.printDocuments", json!({}));
        });
    });
}

/// Preview Results' record number: shows the previewed record; a number typed and Enter go there.
fn record_box(ui: &mut Ui, app: &mut WordApp) {
    let on = !app.session.merge.rows.is_empty();
    let shown = if on { (app.session.merge.record + 1).to_string() } else { String::new() };
    // The draft lives in temp memory while typing (frame-local text would lose keystrokes).
    let key = egui::Id::unique("merge_record_box");
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| shown.clone());
    let r = ui.add_enabled(on, egui::TextEdit::singleline(&mut text).desired_width(36.0).align(egui::Align2::CENTER_TOP));
    let r = r.on_hover_text(tl!("Go to Record"));
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && text.trim() != shown {
            let _ = app.run("mailings.goToRecord", json!({"record": text.trim()}));
        }
    }
}

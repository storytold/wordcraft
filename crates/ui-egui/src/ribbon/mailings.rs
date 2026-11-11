use super::common::{mi, stack, top_row};
use super::language::mi_check;
use super::{Ui, WordApp, big, group, json, menu_button, small};

pub(super) fn mailings(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Create", None, app, |ui, app| {
        // Measured: two rows from 1100 px (880 pt) to 1400 px, one row from 1400 px (1120 pt).
        if crate::ribbon_fit::rows_for(ui.ctx(), "Mailings", "Create", 1) >= 2 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| {
                    small(
                        ui,
                        app,
                        "envelope",
                        Some("Envelopes"),
                        "Envelopes",
                        crate::widgets::ButtonAction::new("mailings.envelopes", json!({}), false),
                    );
                });
                crate::widgets::row(ui, |ui| {
                    small(ui, app, "labels", Some("Labels"), "Labels", crate::widgets::ButtonAction::new("mailings.labels", json!({}), false));
                });
            });
        } else {
            big(ui, app, "envelope", "Envelopes", "mailings.envelopes", json!({}), false);
            big(ui, app, "labels", "Labels", "mailings.labels", json!({}), false);
        }
    });
    group(ui, "Start Mail Merge", None, app, |ui, app| {
        // Measured: one row below 650 px (520 pt), three rows from there, and one row again
        // from 1200 px (960 pt). Each control is its own row in the three-row form.
        let start_merge = |ui: &mut Ui, app: &mut WordApp, big: bool| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "mailMerge",
                    Some(if big { "Start Mail\nMerge" } else { "Start Mail Merge" }),
                    "Start Mail Merge",
                    big,
                    "mailings.start",
                ),
                |ui, app| {
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
                },
            );
        };
        let recipients = |ui: &mut Ui, app: &mut WordApp, big: bool| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "recipients",
                    Some(if big { "Select\nRecipients" } else { "Select Recipients" }),
                    "Select Recipients",
                    big,
                    "mailings.recipients",
                ),
                |ui, app| {
                    mi(ui, app, "Type a New List…", "ui.dialog", json!({"name": "newRecipientList"}));
                    mi(ui, app, "Use an Existing List…", "ui.openRecipientList", json!({}));
                },
            );
        };
        let edit = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "editRecipients", "Edit\nRecipient List", "mailings.editRecipients", json!({}), false);
            } else {
                small(
                    ui,
                    app,
                    "editRecipients",
                    Some("Edit Recipient List"),
                    "Edit Recipient List",
                    crate::widgets::ButtonAction::new("mailings.editRecipients", json!({}), false),
                );
            }
        };
        if crate::ribbon_fit::rows_for(ui.ctx(), "Mailings", "Start Mail Merge", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| start_merge(ui, app, false));
                crate::widgets::row(ui, |ui| recipients(ui, app, false));
                crate::widgets::row(ui, |ui| edit(ui, app, false));
            });
        } else {
            start_merge(ui, app, true);
            recipients(ui, app, true);
            edit(ui, app, true);
        }
    });
    group(ui, "Write & Insert Fields", None, app, |ui, app| {
        let button = |ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, big_btn: bool| {
            if big_btn {
                big(ui, app, icon, label, id, json!({}), false);
            } else {
                let label = label.replace('\n', " ");
                small(ui, app, icon, Some(&label), &label, crate::widgets::ButtonAction::new(id, json!({}), false));
            }
        };
        let highlight = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            button(ui, app, "highlightFields", "Highlight\nMerge Fields", "mailings.highlightFields", big_btn);
        };
        let address = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            button(ui, app, "addressBlock", "Address\nBlock", "mailings.addressBlock", big_btn);
        };
        let greeting = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            button(ui, app, "greetingLine", "Greeting\nLine", "mailings.greetingLine", big_btn);
        };
        let merge_field = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            button(ui, app, "mergeField", "Insert Merge\nField", "mailings.insertField", big_btn);
        };
        let rules = |ui: &mut Ui, app: &mut WordApp| {
            // The merge rules the engine knows; If and Skip Record If ask for their condition.
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("rules", Some("Rules"), "Rules", false, "mailings.rules"), |ui, app| {
                mi(ui, app, "If…Then…Else…", "mailings.rules", json!({"rule": "IF"}));
                mi(ui, app, "Merge Record #", "mailings.rules", json!({"rule": "MERGEREC"}));
                mi(ui, app, "Next Record", "mailings.rules", json!({"rule": "NEXT"}));
                mi(ui, app, "Skip Record If…", "mailings.rules", json!({"rule": "SKIPIF"}));
            });
        };
        let match_fields = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "matchFields",
                Some("Match Fields"),
                "Match Fields",
                crate::widgets::ButtonAction::new("mailings.matchFields", json!({}), false),
            );
        };
        // Measured: one row below 850 px (680 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Mailings", "Write & Insert Fields", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| {
                    highlight(ui, app, false);
                    address(ui, app, false);
                    greeting(ui, app, false);
                });
                crate::widgets::row(ui, |ui| merge_field(ui, app, false));
                crate::widgets::row(ui, |ui| {
                    rules(ui, app);
                    match_fields(ui, app);
                });
            });
        } else {
            top_row(ui, |ui| {
                highlight(ui, app, true);
                address(ui, app, true);
                greeting(ui, app, true);
                merge_field(ui, app, true);
                rules(ui, app);
                match_fields(ui, app);
            });
        }
    });
    group(ui, "Preview Results", None, app, |ui, app| {
        let preview = |ui: &mut Ui, app: &mut WordApp| {
            big(ui, app, "preview", "Preview\nResults", "mailings.preview", json!({}), false);
        };
        let nav = |ui: &mut Ui, app: &mut WordApp| {
            small(ui, app, "first", None, "First Record", crate::widgets::ButtonAction::new("mailings.first", json!({}), false));
            small(ui, app, "previous", None, "Previous Record", crate::widgets::ButtonAction::new("mailings.previous", json!({}), false));
            record_box(ui, app);
            small(ui, app, "next", None, "Next Record", crate::widgets::ButtonAction::new("mailings.next", json!({}), false));
            small(ui, app, "last", None, "Last Record", crate::widgets::ButtonAction::new("mailings.last", json!({}), false));
        };
        let find = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "find",
                Some("Find Recipient"),
                "Find Recipient",
                crate::widgets::ButtonAction::new("mailings.findRecipient", json!({}), false),
            );
        };
        let check = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "checkErrors",
                Some("Check for Errors"),
                "Check for Errors",
                crate::widgets::ButtonAction::new("mailings.checkErrors", json!({}), false),
            );
        };
        // Measured: one row below 1100 px (880 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Mailings", "Preview Results", 1) >= 3 {
            preview(ui, app);
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| nav(ui, app));
                find(ui, app);
                check(ui, app);
            });
        } else {
            top_row(ui, |ui| {
                preview(ui, app);
                nav(ui, app);
                find(ui, app);
                check(ui, app);
            });
        }
    });
    group(ui, "Finish", None, app, |ui, app| {
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("finish", Some("Finish &\nMerge"), "Finish & Merge", true, "mailings.finish"),
            |ui, app| {
                mi(ui, app, "Edit Individual Documents…", "mailings.finish", json!({}));
                mi(ui, app, "Print Documents…", "mailings.printDocuments", json!({}));
            },
        );
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

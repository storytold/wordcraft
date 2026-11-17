use super::common::{mi, stack};
use super::language::language_menu;
use super::{Ui, WordApp, big, big_toggle, group, json, menu_button, small};

pub(super) fn review(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Proofing", None, app, |ui, app| {
        big(ui, app, "spelling", "Spelling &\nGrammar", "review.spelling", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "thesaurus", Some("Thesaurus"), "Thesaurus", crate::widgets::ButtonAction::new("review.thesaurus", json!({}), false));
            small(
                ui,
                app,
                "wordCount",
                Some("Word Count"),
                "Word Count",
                crate::widgets::ButtonAction::new("ui.dialog", json!({"name": "wordCount"}), false),
            );
        });
    });
    group(ui, "Speech", None, app, |ui, app| {
        big(ui, app, "readAloud", "Read\nAloud", "review.readAloud", json!({}), false);
    });
    group(ui, "Accessibility", None, app, |ui, app| {
        big(ui, app, "accessibility", "Check\nAccessibility", "file.accessibility", json!({}), false);
    });
    group(ui, "Language", None, app, |ui, app| {
        big(ui, app, "translate", "Translate", "review.translate", json!({}), false);
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("language", Some("Language"), "Language", true, ""), language_menu);
    });
    group(ui, "Comments", None, app, |ui, app| {
        big(ui, app, "newComment", "New\nComment", "review.newComment", json!({}), false);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "deleteComment",
                Some("Delete"),
                "Delete Comment",
                crate::widgets::ButtonAction::new("review.deleteComment", json!({}), false),
            );
            small(
                ui,
                app,
                "prevComment",
                Some("Previous"),
                "Previous Comment",
                crate::widgets::ButtonAction::new("review.previousComment", json!({}), false),
            );
            small(ui, app, "nextComment", Some("Next"), "Next Comment", crate::widgets::ButtonAction::new("review.nextComment", json!({}), false));
        });
        let shown = app.session.view.comments_pane;
        big(ui, app, "showComments", "Show\nComments", "view.commentsPane", json!({"value": !shown}), false);
    });
    group(ui, "Tracking", Some("review.trackingOptions"), app, |ui, app| {
        let on = app.session.doc.settings.track_changes;
        let r = big(ui, app, "trackChanges", if on { "Track\nChanges ✓" } else { "Track\nChanges" }, "review.trackChanges", json!({}), false);
        let _ = r;
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "markup",
                    Some(if app.session.view.show_markup { "All Markup" } else { "No Markup" }),
                    "Display for Review",
                    false,
                    "review.markup",
                ),
                |ui, app| {
                    mi(ui, app, "All Markup", "review.markup", json!({"value": "all"}));
                    mi(ui, app, "No Markup", "review.markup", json!({"value": "noMarkup"}));
                },
            );
            small(
                ui,
                app,
                "reviewingPane",
                Some("Reviewing Pane"),
                "Reviewing Pane",
                crate::widgets::ButtonAction::new("review.changes", json!({}), false),
            );
        });
    });
    group(ui, "Changes", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("accept", Some("Accept"), "Accept", true, "review.accept"), |ui, app| {
            mi(ui, app, "Accept and Move to Next", "review.accept", json!({"next": true}));
            mi(ui, app, "Accept This Change", "review.accept", json!({}));
            mi(ui, app, "Accept All Changes", "review.acceptAll", json!({}));
            mi(ui, app, "Accept All Changes and Stop Tracking", "review.acceptAll", json!({"stopTracking": true}));
        });
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("reject", Some("Reject"), "Reject", true, "review.reject"), |ui, app| {
            mi(ui, app, "Reject and Move to Next", "review.reject", json!({"next": true}));
            mi(ui, app, "Reject Change", "review.reject", json!({}));
            mi(ui, app, "Reject All Changes", "review.rejectAll", json!({}));
            mi(ui, app, "Reject All Changes and Stop Tracking", "review.rejectAll", json!({"stopTracking": true}));
        });
        stack(ui, |ui| {
            small(
                ui,
                app,
                "prevChange",
                Some("Previous"),
                "Previous Change",
                crate::widgets::ButtonAction::new("review.previousChange", json!({}), false),
            );
            small(ui, app, "nextChange", Some("Next"), "Next Change", crate::widgets::ButtonAction::new("review.nextChange", json!({}), false));
        });
    });
    group(ui, "Compare", None, app, |ui, app| {
        big(ui, app, "compare", "Compare", "review.compare", json!({}), false);
    });
    group(ui, "Protect", None, app, |ui, app| {
        big(ui, app, "blockAuthors", "Block\nAuthors", "review.blockAuthors", json!({}), false);
        big(ui, app, "restrict", "Restrict\nEditing", "review.restrict", json!({}), false);
    });
    group(ui, "Ink", None, app, |ui, app| {
        let hidden = app.session.view.hide_ink;
        big_toggle(ui, app, "hideInk", "Hide\nInk", "review.hideInk", hidden);
    });
}

/// Developer › Controls: content controls.
pub(super) fn developer(app: &mut WordApp, ui: &mut Ui) {
    let design = app.session.view.design_mode;
    group(ui, "Controls", None, app, |ui, app| {
        let rows: [[(&str, &str, &str); 3]; 3] = [
            [
                ("ccRichText", "Rich Text Content Control", "developer.richText"),
                ("ccPlainText", "Plain Text Content Control", "developer.plainText"),
                ("ccPicture", "Picture Content Control", "developer.picture"),
            ],
            [
                ("ccGallery", "Building Block Gallery Content Control", "developer.buildingBlock"),
                ("ccCheckBox", "Check Box Content Control", "developer.checkBox"),
                ("ccComboBox", "Combo Box Content Control", "developer.comboBox"),
            ],
            [
                ("ccDropDown", "Drop-Down List Content Control", "developer.dropDown"),
                ("ccDate", "Date Picker Content Control", "developer.datePicker"),
                ("ccRepeat", "Repeating Section Content Control", "developer.repeatingSection"),
            ],
        ];
        // Three rows of small buttons fill the group's height, like every other stack.
        stack(ui, |ui| {
            for row in rows {
                crate::widgets::row(ui, |ui| {
                    for (icon, tip, id) in row {
                        small(ui, app, icon, None, tip, crate::widgets::ButtonAction::new(id, json!({}), false));
                    }
                });
            }
        });
        stack(ui, |ui| {
            small(
                ui,
                app,
                "designMode",
                Some("Design Mode"),
                "Design Mode",
                crate::widgets::ButtonAction::new("developer.designMode", json!({}), design),
            );
            small(
                ui,
                app,
                "properties",
                Some("Properties"),
                "Properties",
                crate::widgets::ButtonAction::new("developer.properties", json!({}), false),
            );
        });
    });
}

pub(super) fn help(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Help", None, app, |ui, app| {
        big(ui, app, "help", "Help", "ui.dialog", json!({"name": "about"}), false);
        big(ui, app, "checkForUpdates", "Check for\nUpdates…", "ui.checkForUpdates", json!({}), false);
        big(ui, app, "discord", "Community\nDiscord", "ui.discord", json!({}), false);
    });
    group(ui, "Agents", None, app, |ui, app| {
        big(ui, app, "macros", "Commands", "ui.dialog", json!({"name": "commands"}), false);
    });
}

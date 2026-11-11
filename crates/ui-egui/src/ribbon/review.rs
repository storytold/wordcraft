use super::common::{mi, stack, top_row};
use super::language::language_menu;
use super::{Ui, WordApp, big, big_toggle, group, json, menu_button, small};

pub(super) fn review(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Proofing", None, app, |ui, app| {
        let spelling = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "spelling", "Spelling &\nGrammar", "review.spelling", json!({}), false);
            } else {
                small(
                    ui,
                    app,
                    "spelling",
                    Some("Spelling & Grammar"),
                    "Spelling & Grammar",
                    crate::widgets::ButtonAction::new("review.spelling", json!({}), false),
                );
            }
        };
        let thesaurus = |ui: &mut Ui, app: &mut WordApp| {
            small(ui, app, "thesaurus", Some("Thesaurus"), "Thesaurus", crate::widgets::ButtonAction::new("review.thesaurus", json!({}), false));
        };
        let word_count = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "wordCount",
                Some("Word Count"),
                "Word Count",
                crate::widgets::ButtonAction::new("ui.dialog", json!({"name": "wordCount"}), false),
            );
        };
        // Measured: one row below 1161 px (928.8 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Review", "Proofing", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| spelling(ui, app, false));
                crate::widgets::row(ui, |ui| thesaurus(ui, app));
                crate::widgets::row(ui, |ui| word_count(ui, app));
            });
        } else {
            top_row(ui, |ui| {
                spelling(ui, app, true);
                thesaurus(ui, app);
                word_count(ui, app);
            });
        }
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
        let new_comment = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "newComment", "New\nComment", "review.newComment", json!({}), false);
            } else {
                small(
                    ui,
                    app,
                    "newComment",
                    Some("New Comment"),
                    "New Comment",
                    crate::widgets::ButtonAction::new("review.newComment", json!({}), false),
                );
            }
        };
        let delete = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "deleteComment",
                Some("Delete"),
                "Delete Comment",
                crate::widgets::ButtonAction::new("review.deleteComment", json!({}), false),
            );
        };
        let previous = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "prevComment",
                Some("Previous"),
                "Previous Comment",
                crate::widgets::ButtonAction::new("review.previousComment", json!({}), false),
            );
        };
        let next = |ui: &mut Ui, app: &mut WordApp| {
            small(ui, app, "nextComment", Some("Next"), "Next Comment", crate::widgets::ButtonAction::new("review.nextComment", json!({}), false));
        };
        let shown = app.session.view.comments_pane;
        let show_comments = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "showComments", "Show\nComments", "view.commentsPane", json!({"value": !shown}), false);
            } else {
                small(
                    ui,
                    app,
                    "showComments",
                    Some("Show Comments"),
                    "Show Comments",
                    crate::widgets::ButtonAction::new("view.commentsPane", json!({"value": !shown}), false),
                );
            }
        };
        // Measured: one row below 1600 px (1280 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Review", "Comments", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| new_comment(ui, app, false));
                crate::widgets::row(ui, |ui| {
                    delete(ui, app);
                    previous(ui, app);
                });
                crate::widgets::row(ui, |ui| {
                    next(ui, app);
                    show_comments(ui, app, false);
                });
            });
        } else {
            top_row(ui, |ui| {
                new_comment(ui, app, true);
                delete(ui, app);
                previous(ui, app);
                next(ui, app);
                show_comments(ui, app, true);
            });
        }
    });
    group(ui, "Tracking", Some("review.trackingOptions"), app, |ui, app| {
        let on = app.session.doc.settings.track_changes;
        let track = |ui: &mut Ui, app: &mut WordApp| {
            let _ = big(ui, app, "trackChanges", if on { "Track\nChanges ✓" } else { "Track\nChanges" }, "review.trackChanges", json!({}), false);
        };
        let options = |ui: &mut Ui, app: &mut WordApp| {
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
        };
        // Measured: two rows from 1900 px (1520 pt).
        if crate::ribbon_fit::rows_for(ui.ctx(), "Review", "Tracking", 1) >= 2 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| {
                    small(
                        ui,
                        app,
                        "trackChanges",
                        Some("Track Changes"),
                        "Track Changes",
                        crate::widgets::ButtonAction::new("review.trackChanges", json!({}), false),
                    );
                });
                crate::widgets::row(ui, |ui| options(ui, app));
            });
        } else {
            top_row(ui, |ui| {
                track(ui, app);
                options(ui, app);
            });
        }
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
        let button = |ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, params: serde_json::Value, big_btn: bool| {
            if big_btn {
                big(ui, app, icon, label, id, params, false);
            } else {
                let label = label.replace('\n', " ");
                small(ui, app, icon, Some(&label), &label, crate::widgets::ButtonAction::new(id, params, false));
            }
        };
        let help = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            button(ui, app, "help", "Help", "ui.dialog", json!({"name": "about"}), big_btn);
        };
        let updates = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            button(ui, app, "checkForUpdates", "Check for\nUpdates…", "ui.checkForUpdates", json!({}), big_btn);
        };
        let discord = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            button(ui, app, "discord", "Community\nDiscord", "ui.discord", json!({}), big_btn);
        };
        // Measured: three rows below 550 px (440 pt), one row from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Help", "Help", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| help(ui, app, false));
                crate::widgets::row(ui, |ui| updates(ui, app, false));
                crate::widgets::row(ui, |ui| discord(ui, app, false));
            });
        } else {
            top_row(ui, |ui| {
                help(ui, app, true);
                updates(ui, app, true);
                discord(ui, app, true);
            });
        }
    });
    group(ui, "Agents", None, app, |ui, app| {
        big(ui, app, "macros", "Commands", "ui.dialog", json!({"name": "commands"}), false);
    });
}

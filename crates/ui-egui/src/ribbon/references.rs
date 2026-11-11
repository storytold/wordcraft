use super::common::{mi, mi_text, stack, top_row};
use super::language::mi_check;
use super::{Ui, WordApp, big, group, json, menu_button, small};

pub(super) fn references(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table of Contents", None, app, |ui, app| {
        let toc = |ui: &mut Ui, app: &mut WordApp| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("toc", Some("Table of\nContents"), "Table of Contents", true, "references.toc"),
                |ui, app| {
                    mi(ui, app, "Automatic Table 1 (3 levels)", "references.toc", json!({"levels": 3}));
                    mi(ui, app, "Automatic Table 2 (2 levels)", "references.toc", json!({"levels": 2, "title": "Table of Contents"}));
                    ui.separator();
                    mi(ui, app, "Custom Table of Contents…", "ui.dialog", json!({"name": "toc"}));
                    mi(ui, app, "Remove Table of Contents", "references.removeToc", json!({}));
                },
            );
        };
        let add_text = |ui: &mut Ui, app: &mut WordApp| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("addText", Some("Add Text"), "Add Text", false, "references.addText"),
                |ui, app| {
                    mi(ui, app, "Do Not Show in TOC", "references.addText", json!({"level": 0}));
                    for l in 1..=3u64 {
                        mi(ui, app, &format!("Level {l}"), "references.addText", json!({"level": l}));
                    }
                },
            );
        };
        let update = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "updateTable",
                Some("Update Table"),
                "Update Table",
                crate::widgets::ButtonAction::new("references.updateToc", json!({}), false),
            );
        };
        // Measured: one row below 750 px (600 pt), two rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "References", "Table of Contents", 1) >= 2 {
            toc(ui, app);
            stack(ui, |ui| {
                add_text(ui, app);
                update(ui, app);
            });
        } else {
            top_row(ui, |ui| {
                toc(ui, app);
                add_text(ui, app);
                update(ui, app);
            });
        }
    });
    group(ui, "Footnotes", Some("references.noteOptions"), app, |ui, app| {
        let footnote = |ui: &mut Ui, app: &mut WordApp| {
            big(ui, app, "footnote", "Insert\nFootnote", "references.footnote", json!({}), false);
        };
        let endnote = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "endnote",
                Some("Insert Endnote"),
                "Insert Endnote",
                crate::widgets::ButtonAction::new("references.endnote", json!({}), false),
            );
        };
        let next = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "nextFootnote",
                Some("Next Footnote"),
                "Next Footnote",
                crate::widgets::ButtonAction::new("references.nextFootnote", json!({}), false),
            );
        };
        let show = |ui: &mut Ui, app: &mut WordApp| {
            small(ui, app, "showNotes", Some("Show Notes"), "Show Notes", crate::widgets::ButtonAction::new("references.notes", json!({}), false));
        };
        // Measured: one row below 750 px (600 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "References", "Footnotes", 1) >= 3 {
            footnote(ui, app);
            stack(ui, |ui| {
                endnote(ui, app);
                next(ui, app);
                show(ui, app);
            });
        } else {
            top_row(ui, |ui| {
                footnote(ui, app);
                endnote(ui, app);
                next(ui, app);
                show(ui, app);
            });
        }
    });
    group(ui, "Research", None, app, |ui, app| {
        big(ui, app, "researcher", "Researcher", "references.researcher", json!({}), false);
    });
    group(ui, "Citations & Bibliography", None, app, |ui, app| {
        let citation = |ui: &mut Ui, app: &mut WordApp| {
            // The document's sources to cite, then a new one (Create Source, #402).
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("citation", Some("Insert\nCitation"), "Insert Citation", true, ""),
                |ui, app| {
                    let sources: Vec<(String, String)> =
                        app.session.doc.sources.iter().map(|s| (s.tag.clone(), format!("{} — {}", s.author, s.title))).collect();
                    for (tag, label) in sources {
                        mi_text(ui, app, &label, "references.citation", json!({"tag": tag}));
                    }
                    if !app.session.doc.sources.is_empty() {
                        ui.separator();
                    }
                    mi(ui, app, "Add New Source…", "ui.dialog", json!({"name": "createSource"}));
                },
            );
        };
        let sources = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "sources",
                Some("Manage Sources"),
                "Manage Sources",
                crate::widgets::ButtonAction::new("references.sources", json!({}), false),
            );
        };
        let style = |ui: &mut Ui, app: &mut WordApp| {
            let current = app.session.bib_style.clone();
            let label = crate::i18n::fmt(tl!("Style: {style}"), &[("style", &current)]);
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("styles", Some(&label), "Citation Style", false, "references.citationStyle"),
                |ui, app| {
                    for st in &wordcraft_engine::cmd::citations::STYLES {
                        mi_check(ui, app, st.name, st.name == current, "references.citationStyle", json!({"style": st.name}));
                    }
                },
            );
        };
        let bibliography = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "bibliography",
                Some("Bibliography"),
                "Bibliography",
                crate::widgets::ButtonAction::new("references.bibliography", json!({}), false),
            );
        };
        // Measured: one row below 950 px (760 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "References", "Citations & Bibliography", 1) >= 3 {
            citation(ui, app);
            stack(ui, |ui| {
                sources(ui, app);
                style(ui, app);
                bibliography(ui, app);
            });
        } else {
            top_row(ui, |ui| {
                citation(ui, app);
                sources(ui, app);
                style(ui, app);
                bibliography(ui, app);
            });
        }
    });
    group(ui, "Captions", None, app, |ui, app| {
        let caption = |ui: &mut Ui, app: &mut WordApp| {
            big(ui, app, "caption", "Insert\nCaption", "references.caption", json!({}), false);
        };
        let figures = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "tableOfFigures",
                Some("Insert Table of Figures"),
                "Insert Table of Figures",
                crate::widgets::ButtonAction::new("references.tableOfFigures", json!({}), false),
            );
        };
        let update = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "update",
                Some("Update Table"),
                "Update Table of Figures",
                crate::widgets::ButtonAction::new("references.updateFigures", json!({}), false),
            );
        };
        let cross_ref = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "crossRef",
                Some("Cross-reference"),
                "Cross-reference",
                crate::widgets::ButtonAction::new("insert.crossReference", json!({}), false),
            );
        };
        // Measured: one row below 1024 px (819.2 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "References", "Captions", 1) >= 3 {
            caption(ui, app);
            stack(ui, |ui| {
                figures(ui, app);
                update(ui, app);
                cross_ref(ui, app);
            });
        } else {
            top_row(ui, |ui| {
                caption(ui, app);
                figures(ui, app);
                update(ui, app);
                cross_ref(ui, app);
            });
        }
    });
    group(ui, "Index", None, app, |ui, app| {
        let insert = |ui: &mut Ui, app: &mut WordApp| {
            small(ui, app, "index", Some("Insert Index"), "Insert Index", crate::widgets::ButtonAction::new("references.index", json!({}), false));
            small(
                ui,
                app,
                "update",
                Some("Update Index"),
                "Update Index",
                crate::widgets::ButtonAction::new("references.updateIndex", json!({}), false),
            );
        };
        // Measured: two rows from 1024 px (819.2 pt), the first width it was seen at.
        if crate::ribbon_fit::rows_for(ui.ctx(), "References", "Index", 1) >= 2 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| {
                    small(
                        ui,
                        app,
                        "markEntry",
                        Some("Mark Entry"),
                        "Mark Entry",
                        crate::widgets::ButtonAction::new("references.markEntry", json!({}), false),
                    );
                });
                crate::widgets::row(ui, |ui| insert(ui, app));
            });
        } else {
            top_row(ui, |ui| {
                big(ui, app, "markEntry", "Mark\nEntry", "references.markEntry", json!({}), false);
                insert(ui, app);
            });
        }
    });
    group(ui, "Table of Authorities", None, app, |ui, app| {
        let insert = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "tableOfAuthorities",
                Some("Insert Table of Authorities"),
                "Insert Table of Authorities",
                crate::widgets::ButtonAction::new("references.tableOfAuthorities", json!({}), false),
            );
            small(
                ui,
                app,
                "update",
                Some("Update Table"),
                "Update Table",
                crate::widgets::ButtonAction::new("references.updateAuthorities", json!({}), false),
            );
        };
        // Measured: two rows from 1024 px (819.2 pt), as the Index.
        if crate::ribbon_fit::rows_for(ui.ctx(), "References", "Table of Authorities", 1) >= 2 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| {
                    small(
                        ui,
                        app,
                        "markCitation",
                        Some("Mark Citation"),
                        "Mark Citation",
                        crate::widgets::ButtonAction::new("references.markCitation", json!({}), false),
                    );
                });
                crate::widgets::row(ui, |ui| insert(ui, app));
            });
        } else {
            top_row(ui, |ui| {
                big(ui, app, "markCitation", "Mark\nCitation", "references.markCitation", json!({}), false);
                insert(ui, app);
            });
        }
    });
}

/// Zotero (`docs/zotero.md`): the Zotero desktop app does the work in its own window.
pub(super) fn zotero(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Citations", None, app, |ui, app| {
        big(ui, app, "citation", "Add/Edit\nCitation", "ui.zotero.addEditCitation", json!({}), false);
        big(ui, app, "addNote", "Add\nNote", "ui.zotero.addNote", json!({}), false);
        big(ui, app, "pastCitation", "Move Past\nCitation", "caret.pastCitation", json!({}), false);
    });
    group(ui, "Bibliography", None, app, |ui, app| {
        big(ui, app, "bibliography", "Add/Edit\nBibliography", "ui.zotero.addEditBibliography", json!({}), false);
    });
    group(ui, "Document", None, app, |ui, app| {
        big(ui, app, "update", "Refresh", "ui.zotero.refresh", json!({}), false);
        big(ui, app, "docPrefs", "Document\nPreferences", "ui.zotero.setDocPrefs", json!({}), false);
        big(ui, app, "unlinkCitations", "Unlink\nCitations", "ui.zotero.removeCodes", json!({}), false);
    });
}

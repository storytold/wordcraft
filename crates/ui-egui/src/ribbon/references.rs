use super::common::{mi, mi_text, stack};
use super::language::{mi_check, mi_check_text};
use super::{Ui, WordApp, big, group, json, menu_button, small};

/// The name the References ▸ Style combo shows for the document's citation style: a built-in's
/// name, or the title of the CSL style a `csl:<id>` choice stands for (the id itself when that
/// style is no longer installed).
fn citation_style_name(app: &WordApp, bib_style: &str) -> String {
    let Some(id) = bib_style.strip_prefix("csl:") else { return bib_style.to_string() };
    app.session
        .csl_styles
        .iter()
        .find(|e| e.id().eq_ignore_ascii_case(id))
        .map_or_else(|| bib_style.to_string(), |e| if e.style.title.is_empty() { e.file.clone() } else { e.style.title.clone() })
}

pub(super) fn references(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table of Contents", None, app, |ui, app| {
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
        stack(ui, |ui| {
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
            small(
                ui,
                app,
                "updateTable",
                Some("Update Table"),
                "Update Table",
                crate::widgets::ButtonAction::new("references.updateToc", json!({}), false),
            );
        });
    });
    group(ui, "Footnotes", Some("references.noteOptions"), app, |ui, app| {
        big(ui, app, "footnote", "Insert\nFootnote", "references.footnote", json!({}), false);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "endnote",
                Some("Insert Endnote"),
                "Insert Endnote",
                crate::widgets::ButtonAction::new("references.endnote", json!({}), false),
            );
            small(
                ui,
                app,
                "nextFootnote",
                Some("Next Footnote"),
                "Next Footnote",
                crate::widgets::ButtonAction::new("references.nextFootnote", json!({}), false),
            );
            small(ui, app, "showNotes", Some("Show Notes"), "Show Notes", crate::widgets::ButtonAction::new("references.notes", json!({}), false));
        });
    });
    group(ui, "Research", None, app, |ui, app| {
        big(ui, app, "researcher", "Researcher", "references.researcher", json!({}), false);
    });
    group(ui, "Citations & Bibliography", None, app, |ui, app| {
        // The document's sources to cite, then a new one (Create Source, #402).
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("citation", Some("Insert\nCitation"), "Insert Citation", true, ""), |ui, app| {
            let sources: Vec<(String, String)> =
                app.session.doc.sources.iter().map(|s| (s.tag.clone(), format!("{} — {}", s.author, s.title))).collect();
            for (tag, label) in sources {
                mi_text(ui, app, &label, "references.citation", json!({"tag": tag}));
            }
            if !app.session.doc.sources.is_empty() {
                ui.separator();
            }
            mi(ui, app, "Add New Source…", "ui.dialog", json!({"name": "createSource"}));
        });
        stack(ui, |ui| {
            small(
                ui,
                app,
                "sources",
                Some("Manage Sources"),
                "Manage Sources",
                crate::widgets::ButtonAction::new("references.sources", json!({}), false),
            );
            let current = app.session.bib_style.clone();
            let shown = citation_style_name(app, &current);
            let label = crate::i18n::fmt(tl!("Style: {style}"), &[("style", &shown)]);
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "styles",
                    Some(&label),
                    "Citation Style (CSL styles: docs/csl-styles.md)",
                    false,
                    "references.citationStyle",
                ),
                |ui, app| {
                    for st in &wordcraft_engine::cmd::citations::STYLES {
                        mi_check(ui, app, st.name, st.name == current, "references.citationStyle", json!({"style": st.name}));
                    }
                    // #1058: styles added from CSL files, by title; the document's recorded choice is
                    // `csl:<id>`.
                    let csl: Vec<(String, String)> = app
                        .session
                        .csl_styles
                        .iter()
                        .map(|e| (e.id().to_string(), if e.style.title.is_empty() { e.file.clone() } else { e.style.title.clone() }))
                        .collect();
                    if !csl.is_empty() {
                        ui.separator();
                    }
                    for (id, title) in &csl {
                        let on = current.strip_prefix("csl:").is_some_and(|c| c.eq_ignore_ascii_case(id));
                        mi_check_text(ui, app, title, on, "references.citationStyle", json!({"style": format!("csl:{id}")}));
                    }
                    ui.separator();
                    // #1058: pick a CSL file to install as a style; it is added, its warnings show in
                    // the status line, and it becomes the document's style.
                    if ui.button(tl!("Add style from file…")).clicked() {
                        let _ = app.ask_file(
                            crate::file_dialogs::FileDialogRequest::Open { purpose: "csl-style".into() },
                            crate::file_dialogs::AfterPick::CslStyle,
                        );
                        ui.close();
                    }
                },
            );
            small(
                ui,
                app,
                "bibliography",
                Some("Bibliography"),
                "Bibliography",
                crate::widgets::ButtonAction::new("references.bibliography", json!({}), false),
            );
        });
    });
    group(ui, "Captions", None, app, |ui, app| {
        big(ui, app, "caption", "Insert\nCaption", "references.caption", json!({}), false);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "tableOfFigures",
                Some("Insert Table of Figures"),
                "Insert Table of Figures",
                crate::widgets::ButtonAction::new("references.tableOfFigures", json!({}), false),
            );
            small(
                ui,
                app,
                "update",
                Some("Update Table"),
                "Update Table of Figures",
                crate::widgets::ButtonAction::new("references.updateFigures", json!({}), false),
            );
            small(
                ui,
                app,
                "crossRef",
                Some("Cross-reference"),
                "Cross-reference",
                crate::widgets::ButtonAction::new("insert.crossReference", json!({}), false),
            );
        });
    });
    group(ui, "Index", None, app, |ui, app| {
        big(ui, app, "markEntry", "Mark\nEntry", "references.markEntry", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "index", Some("Insert Index"), "Insert Index", crate::widgets::ButtonAction::new("references.index", json!({}), false));
            small(
                ui,
                app,
                "update",
                Some("Update Index"),
                "Update Index",
                crate::widgets::ButtonAction::new("references.updateIndex", json!({}), false),
            );
        });
    });
    group(ui, "Table of Authorities", None, app, |ui, app| {
        big(ui, app, "markCitation", "Mark\nCitation", "references.markCitation", json!({}), false);
        stack(ui, |ui| {
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
        });
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

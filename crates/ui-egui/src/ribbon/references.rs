use super::common::{mi, mi_text, stack};
use super::language::mi_check;
use super::*;

pub(super) fn references(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table of Contents", None, app, |ui, app| {
        menu_button(ui, app, "toc", Some("Table of\nContents"), "Table of Contents", true, "references.toc", |ui, app| {
            mi(ui, app, "Automatic Table 1 (3 levels)", "references.toc", json!({"levels": 3}));
            mi(ui, app, "Automatic Table 2 (2 levels)", "references.toc", json!({"levels": 2, "title": "Table of Contents"}));
            ui.separator();
            mi(ui, app, "Custom Table of Contents…", "ui.dialog", json!({"name": "toc"}));
            mi(ui, app, "Remove Table of Contents", "references.removeToc", json!({}));
        });
        stack(ui, |ui| {
            menu_button(ui, app, "addText", Some("Add Text"), "Add Text", false, "references.addText", |ui, app| {
                mi(ui, app, "Do Not Show in TOC", "references.addText", json!({"level": 0}));
                for l in 1..=3u64 {
                    mi(ui, app, &format!("Level {l}"), "references.addText", json!({"level": l}));
                }
            });
            small(ui, app, "updateTable", Some("Update Table"), "Update Table", "references.updateToc", json!({}), false);
        });
    });
    group(ui, "Footnotes", Some("references.noteOptions"), app, |ui, app| {
        big(ui, app, "footnote", "Insert\nFootnote", "references.footnote", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "endnote", Some("Insert Endnote"), "Insert Endnote", "references.endnote", json!({}), false);
            small(ui, app, "nextFootnote", Some("Next Footnote"), "Next Footnote", "references.nextFootnote", json!({}), false);
            small(ui, app, "showNotes", Some("Show Notes"), "Show Notes", "references.notes", json!({}), false);
        });
    });
    group(ui, "Research", None, app, |ui, app| {
        big(ui, app, "researcher", "Researcher", "references.researcher", json!({}), false);
    });
    group(ui, "Citations & Bibliography", None, app, |ui, app| {
        // The document's sources to cite, then a new one (Create Source, #402).
        menu_button(ui, app, "citation", Some("Insert\nCitation"), "Insert Citation", true, "", |ui, app| {
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
            small(ui, app, "sources", Some("Manage Sources"), "Manage Sources", "references.sources", json!({}), false);
            let current = app.session.bib_style.clone();
            let label = crate::i18n::fmt(tl!("Style: {style}"), &[("style", &current)]);
            menu_button(ui, app, "styles", Some(&label), "Citation Style", false, "references.citationStyle", |ui, app| {
                for st in &wordcraft_engine::cmd::citations::STYLES {
                    mi_check(ui, app, st.name, st.name == current, "references.citationStyle", json!({"style": st.name}));
                }
            });
            small(ui, app, "bibliography", Some("Bibliography"), "Bibliography", "references.bibliography", json!({}), false);
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
                "references.tableOfFigures",
                json!({}),
                false,
            );
            small(ui, app, "update", Some("Update Table"), "Update Table of Figures", "references.updateFigures", json!({}), false);
            small(ui, app, "crossRef", Some("Cross-reference"), "Cross-reference", "insert.crossReference", json!({}), false);
        });
    });
    group(ui, "Index", None, app, |ui, app| {
        big(ui, app, "markEntry", "Mark\nEntry", "references.markEntry", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "index", Some("Insert Index"), "Insert Index", "references.index", json!({}), false);
            small(ui, app, "update", Some("Update Index"), "Update Index", "references.updateIndex", json!({}), false);
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
                "references.tableOfAuthorities",
                json!({}),
                false,
            );
            small(ui, app, "update", Some("Update Table"), "Update Table", "references.updateAuthorities", json!({}), false);
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

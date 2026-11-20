use super::common::menu_button;
use super::{WordApp, json};
use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button};
use craft::ui::contract::{Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::layout_components::Stack;
use craft::ui::menu::MenuItem;

pub(super) fn references(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(RibbonGroup::new("Table of Contents", crate::tl!("Table of Contents"), IconId::new("toc").into()), |ui| {
        menu_button(
            ui,
            Button::new(crate::tl!("Table of\nContents"))
                .icon(IconId::new("toc"))
                .tooltip(crate::tl!("Table of Contents"))
                .arrangement(Arrangement::Stacked),
            crate::tl!("Table of Contents"),
            Some("references.toc"),
            |ui, _| {
                for (label, params) in [
                    ("Automatic Table 1 (3 levels)", json!({"levels": 3})),
                    ("Automatic Table 2 (2 levels)", json!({"levels": 2, "title": "Table of Contents"})),
                ] {
                    MenuItem::command(CommandRef::with_params("references.toc", params)).label(crate::tl!(label)).show(ui);
                }
                Divider::horizontal().show(ui);
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "toc"})))
                    .label(crate::tl!("Custom Table of Contents…"))
                    .show(ui);
                MenuItem::command(CommandRef::with_params("references.removeToc", json!({}))).label(crate::tl!("Remove Table of Contents")).show(ui);
            },
        );
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(
                ui,
                Button::new(crate::tl!("Add Text")).icon(IconId::new("addText")).tooltip(crate::tl!("Add Text")),
                crate::tl!("Add Text"),
                Some("references.addText"),
                |ui, _| {
                    MenuItem::command(CommandRef::with_params("references.addText", json!({"level": 0})))
                        .label(crate::tl!("Do Not Show in TOC"))
                        .show(ui);
                    for l in 1..=3u64 {
                        let label = format!("Level {l}");
                        MenuItem::command(CommandRef::with_params("references.addText", json!({"level": l})))
                            .label(crate::tl!(&label).to_string())
                            .show(ui);
                    }
                },
            );
            Button::command(CommandRef::with_params("references.updateToc", json!({})))
                .label(crate::tl!("Update Table"))
                .icon(IconId::new("updateTable"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("Footnotes", crate::tl!("Footnotes"), IconId::new("footnote").into()).launcher("references.noteOptions"), |ui| {
        Button::command(CommandRef::with_params("references.footnote", json!({})))
            .label(crate::tl!("Insert\nFootnote"))
            .icon(IconId::new("footnote"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for (key, label, id) in [
                ("endnote", "Insert Endnote", "references.endnote"),
                ("nextFootnote", "Next Footnote", "references.nextFootnote"),
                ("showNotes", "Show Notes", "references.notes"),
            ] {
                Button::command(CommandRef::with_params(id, json!({}))).label(crate::tl!(label)).icon(crate::icons::id(key)).show(ui);
            }
        });
    });
    row.group(RibbonGroup::new("Research", crate::tl!("Research"), IconId::new("researcher").into()), |ui| {
        Button::command(CommandRef::with_params("references.researcher", json!({})))
            .label(crate::tl!("Researcher"))
            .icon(IconId::new("researcher"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("Citations & Bibliography", crate::tl!("Citations & Bibliography"), IconId::new("citation").into()), |ui| {
        // The document's sources to cite, then a new one (Create Source, #402).
        menu_button(
            ui,
            Button::new(crate::tl!("Insert\nCitation"))
                .icon(IconId::new("citation"))
                .tooltip(crate::tl!("Insert Citation"))
                .arrangement(Arrangement::Stacked),
            crate::tl!("Insert Citation"),
            Some("references.citation"),
            |ui, _| {
                for s in &app.session.doc.sources {
                    let label = format!("{} — {}", s.author, s.title);
                    MenuItem::command(CommandRef::with_params("references.citation", json!({"tag": s.tag}))).label(label).show(ui);
                }
                if !app.session.doc.sources.is_empty() {
                    Divider::horizontal().show(ui);
                }
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "createSource"})))
                    .label(crate::tl!("Add New Source…"))
                    .show(ui);
            },
        );
        let current = app.session.bib_style.clone();
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::with_params("references.sources", json!({})))
                .label(crate::tl!("Manage Sources"))
                .icon(IconId::new("sources"))
                .show(ui);
            let label = crate::i18n::fmt(crate::tl!("Style: {style}"), &[("style", &current)]);
            menu_button(
                ui,
                Button::new(label).icon(IconId::new("styles")).tooltip(crate::tl!("Citation Style")),
                crate::tl!("Citation Style"),
                Some("references.citationStyle"),
                |ui, _| {
                    for st in &wordcraft_engine::cmd::citations::STYLES {
                        MenuItem::command(CommandRef::with_params("references.citationStyle", json!({"style": st.name})))
                            .label(crate::tl!(st.name))
                            .radio_mark(st.name == current)
                            .show(ui);
                    }
                },
            );
            Button::command(CommandRef::with_params("references.bibliography", json!({})))
                .label(crate::tl!("Bibliography"))
                .icon(IconId::new("bibliography"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("Captions", crate::tl!("Captions"), IconId::new("caption").into()), |ui| {
        Button::command(CommandRef::with_params("references.caption", json!({})))
            .label(crate::tl!("Insert\nCaption"))
            .icon(IconId::new("caption"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for (key, label, tip, id) in [
                ("tableOfFigures", "Insert Table of Figures", "Insert Table of Figures", "references.tableOfFigures"),
                ("update", "Update Table", "Update Table of Figures", "references.updateFigures"),
                ("crossRef", "Cross-reference", "Cross-reference", "insert.crossReference"),
            ] {
                Button::command(CommandRef::with_params(id, json!({})))
                    .label(crate::tl!(label))
                    .icon(crate::icons::id(key))
                    .tooltip(crate::tl!(tip))
                    .show(ui);
            }
        });
    });
    row.group(RibbonGroup::new("Index", crate::tl!("Index"), IconId::new("index").into()), |ui| {
        Button::command(CommandRef::with_params("references.markEntry", json!({})))
            .label(crate::tl!("Mark\nEntry"))
            .icon(IconId::new("markEntry"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for (key, label, id) in [("index", "Insert Index", "references.index"), ("update", "Update Index", "references.updateIndex")] {
                Button::command(CommandRef::with_params(id, json!({}))).label(crate::tl!(label)).icon(crate::icons::id(key)).show(ui);
            }
        });
    });
    row.group(RibbonGroup::new("Table of Authorities", crate::tl!("Table of Authorities"), IconId::new("tableOfAuthorities").into()), |ui| {
        Button::command(CommandRef::with_params("references.markCitation", json!({})))
            .label(crate::tl!("Mark\nCitation"))
            .icon(IconId::new("markCitation"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for (key, label, id) in [
                ("tableOfAuthorities", "Insert Table of Authorities", "references.tableOfAuthorities"),
                ("update", "Update Table", "references.updateAuthorities"),
            ] {
                Button::command(CommandRef::with_params(id, json!({}))).label(crate::tl!(label)).icon(crate::icons::id(key)).show(ui);
            }
        });
    });
}

/// Zotero (`docs/zotero.md`): the Zotero desktop app does the work in its own window.
pub(super) fn zotero(row: &mut RibbonRow<'_>, _app: &mut WordApp) {
    let big = |ui: &mut egui::Ui, key: &'static str, label: &'static str, id: &'static str| {
        Button::command(CommandRef::with_params(id, json!({})))
            .label(crate::tl!(label))
            .icon(crate::icons::id(key))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    };
    row.group(RibbonGroup::new("Provider", crate::tl!("Provider"), IconId::new("citation").into()), |ui| {
        menu_button(
            ui,
            Button::new(crate::tl!("Provider")).icon(IconId::new("citation")).tooltip(crate::tl!("Citation Provider")),
            crate::tl!("Citation Provider"),
            Some("ui.citations.provider"),
            |ui, _| {
                MenuItem::command(CommandRef::with_params("ui.citations.provider", json!({"value": "zotero"})))
                    .label(crate::tl!("Zotero"))
                    .radio_mark(true)
                    .show(ui);
            },
        );
    });
    row.group(RibbonGroup::new("Citations", crate::tl!("Citations"), IconId::new("citation").into()), |ui| {
        big(ui, "citation", "Add/Edit\nCitation", "ui.zotero.addEditCitation");
        big(ui, "addNote", "Add\nNote", "ui.zotero.addNote");
        big(ui, "pastCitation", "Move Past\nCitation", "caret.pastCitation");
    });
    row.group(RibbonGroup::new("Bibliography", crate::tl!("Bibliography"), IconId::new("bibliography").into()), |ui| {
        big(ui, "bibliography", "Add/Edit\nBibliography", "ui.zotero.addEditBibliography");
    });
    row.group(RibbonGroup::new("Document", crate::tl!("Document"), IconId::new("docPrefs").into()), |ui| {
        big(ui, "update", "Refresh", "ui.zotero.refresh");
        big(ui, "docPrefs", "Document\nPreferences", "ui.zotero.setDocPrefs");
        big(ui, "unlinkCitations", "Unlink\nCitations", "ui.zotero.removeCodes");
    });
}

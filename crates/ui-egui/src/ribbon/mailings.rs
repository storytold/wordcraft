use super::common::menu_button;
use super::{WordApp, json};
use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button, IconButton};
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::MenuItem;
use craft::ui::text::NumberField;

pub(super) fn mailings(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(RibbonGroup::new("Create", crate::tl!("Create"), IconId::new("envelope").into()), |ui| {
        for (key, label, id) in [("envelope", "Envelopes", "mailings.envelopes"), ("labels", "Labels", "mailings.labels")] {
            Button::command(CommandRef::with_params(id, json!({})))
                .label(crate::tl!(label))
                .icon(crate::icons::spec(key))
                .arrangement(Arrangement::Stacked)
                .show(ui);
        }
    });
    row.group(RibbonGroup::new("Start Mail Merge", crate::tl!("Start Mail Merge"), IconId::new("mailMerge").into()), |ui| {
        // The kinds of merge document the engine makes, the current one marked.
        menu_button(
            ui,
            Button::new(crate::tl!("Start Mail\nMerge"))
                .icon(IconId::new("mailMerge"))
                .tooltip(crate::tl!("Start Mail Merge"))
                .arrangement(Arrangement::Stacked),
            crate::tl!("Start Mail Merge"),
            Some("mailings.start"),
            |ui, _| {
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
                    MenuItem::command(CommandRef::with_params("mailings.start", json!({"kind": k})))
                        .label(crate::tl!(label))
                        .radio_mark(current)
                        .show(ui);
                }
            },
        );
        // The command needs data, so the button offers the two ways to give it (#240).
        menu_button(
            ui,
            Button::new(crate::tl!("Select\nRecipients"))
                .icon(IconId::new("recipients"))
                .tooltip(crate::tl!("Select Recipients"))
                .arrangement(Arrangement::Stacked),
            crate::tl!("Select Recipients"),
            None,
            |ui, _| {
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "newRecipientList"})))
                    .label(crate::tl!("Type a New List…"))
                    .show(ui);
                MenuItem::command(CommandRef::with_params("ui.openRecipientList", json!({}))).label(crate::tl!("Use an Existing List…")).show(ui);
            },
        );
        Button::command(CommandRef::with_params("mailings.editRecipients", json!({})))
            .label(crate::tl!("Edit\nRecipient List"))
            .icon(IconId::new("editRecipients"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("Write & Insert Fields", crate::tl!("Write & Insert Fields"), crate::icons::spec("mergeField")), |ui| {
        for (key, label, id) in [
            ("highlightFields", "Highlight\nMerge Fields", "mailings.highlightFields"),
            ("addressBlock", "Address\nBlock", "mailings.addressBlock"),
            ("greetingLine", "Greeting\nLine", "mailings.greetingLine"),
            ("mergeField", "Insert Merge\nField", "mailings.insertField"),
        ] {
            Button::command(CommandRef::with_params(id, json!({})))
                .label(crate::tl!(label))
                .icon(crate::icons::spec(key))
                .arrangement(Arrangement::Stacked)
                .show(ui);
        }
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            // The merge rules the engine knows; If and Skip Record If ask for their condition.
            menu_button(
                ui,
                Button::new(crate::tl!("Rules")).icon(crate::icons::spec("rules")).tooltip(crate::tl!("Rules")),
                crate::tl!("Rules"),
                Some("mailings.rules"),
                |ui, _| {
                    for (label, rule) in
                        [("If…Then…Else…", "IF"), ("Merge Record #", "MERGEREC"), ("Next Record", "NEXT"), ("Skip Record If…", "SKIPIF")]
                    {
                        MenuItem::command(CommandRef::with_params("mailings.rules", json!({"rule": rule}))).label(crate::tl!(label)).show(ui);
                    }
                },
            );
            Button::command(CommandRef::with_params("mailings.matchFields", json!({})))
                .label(crate::tl!("Match Fields"))
                .icon(crate::icons::spec("matchFields"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("Preview Results", crate::tl!("Preview Results"), crate::icons::spec("preview")), |ui| {
        Button::command(CommandRef::with_params("mailings.preview", json!({})))
            .label(crate::tl!("Preview\nResults"))
            .icon(crate::icons::spec("preview"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let nav = |ui: &mut egui::Ui, key: &'static str, tip: &'static str, id: &'static str| {
                    IconButton::command(crate::icons::spec(key), CommandRef::with_params(id, json!({})))
                        .label(crate::tl!(tip))
                        .tooltip(crate::tl!(tip))
                        .show(ui);
                };
                nav(ui, "first", "First Record", "mailings.first");
                nav(ui, "previous", "Previous Record", "mailings.previous");
                record_box(ui, app);
                nav(ui, "next", "Next Record", "mailings.next");
                nav(ui, "last", "Last Record", "mailings.last");
            });
            Button::command(CommandRef::with_params("mailings.findRecipient", json!({})))
                .label(crate::tl!("Find Recipient"))
                .icon(IconId::new("find"))
                .show(ui);
            Button::command(CommandRef::with_params("mailings.checkErrors", json!({})))
                .label(crate::tl!("Check for Errors"))
                .icon(IconId::new("checkErrors"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("Finish", crate::tl!("Finish"), IconId::new("finish").into()), |ui| {
        menu_button(
            ui,
            Button::new(crate::tl!("Finish &\nMerge"))
                .icon(IconId::new("finish"))
                .tooltip(crate::tl!("Finish & Merge"))
                .arrangement(Arrangement::Stacked),
            crate::tl!("Finish & Merge"),
            Some("mailings.finish"),
            |ui, _| {
                MenuItem::command(CommandRef::with_params("mailings.finish", json!({}))).label(crate::tl!("Edit Individual Documents…")).show(ui);
                MenuItem::command(CommandRef::with_params("mailings.printDocuments", json!({}))).label(crate::tl!("Print Documents…")).show(ui);
            },
        );
    });
}

/// Preview Results' record number: shows the previewed record; Enter on a different number goes there.
/// It navigates and does not edit, so a keystroke runs nothing.
fn record_box(ui: &mut egui::Ui, app: &mut WordApp) {
    let on = !app.session.merge.rows.is_empty();
    let shown = on.then(|| (app.session.merge.record + 1) as f64);
    let out = NumberField::new(crate::tl!("Record"), Bind::given(shown))
        .enabled(on)
        .range(1.0..=app.session.merge.rows.len().max(1) as f64)
        .tooltip(crate::tl!("Go to Record"))
        .show(ui);
    if out.edit.submitted
        && let Some(Some(n)) = out.edit.value().copied()
        && Some(n) != shown
    {
        let _ = app.run("mailings.goToRecord", json!({"record": (n.round() as i64).to_string()}));
    }
}

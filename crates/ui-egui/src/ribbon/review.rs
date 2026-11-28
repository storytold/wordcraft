use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::ribbon::{RibbonGroup, RibbonRow}; // PLAN: assumed ribbon-w8 adds the craft-menu dependency and passes the `RibbonRow`
use craft::ui::buttons::{Arrangement, Button, IconButton, SplitButton, SplitMenu, ToggleButton};
use craft::ui::contract::{Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::MenuItem;

use super::common::menu_button;
use super::language::language_menu;
use super::{WordApp, json};

/// The Review tab.
pub(super) fn review(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(RibbonGroup::new("review.proofing", tl!("Proofing").to_string(), crate::icons::spec("spelling")), |ui| {
        Button::command(CommandRef::new("review.spelling"))
            .label(tl!("Spelling &\nGrammar").to_string())
            .icon(crate::icons::spec("spelling"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("review.thesaurus")).label(tl!("Thesaurus").to_string()).icon(crate::icons::spec("thesaurus")).show(ui);
            Button::command(CommandRef::with_params("ui.dialog", json!({"name": "wordCount"})))
                .label(tl!("Word Count").to_string())
                .icon(crate::icons::spec("wordCount"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("review.speech", tl!("Speech").to_string(), crate::icons::spec("readAloud")), |ui| {
        Button::command(CommandRef::new("review.readAloud"))
            .label(tl!("Read\nAloud").to_string())
            .icon(crate::icons::spec("readAloud"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("review.accessibility", tl!("Accessibility").to_string(), IconId::new("accessibility").into()), |ui| {
        Button::command(CommandRef::new("file.accessibility"))
            .label(tl!("Check\nAccessibility").to_string())
            .icon(IconId::new("accessibility"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("review.language", tl!("Language").to_string(), crate::icons::spec("language")), |ui| {
        Button::command(CommandRef::new("review.translate"))
            .label(tl!("Translate").to_string())
            .icon(crate::icons::spec("translate"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        let face = Button::new(tl!("Language").to_string()).icon(crate::icons::spec("language")).arrangement(Arrangement::Stacked);
        menu_button(ui, face, tl!("Language").to_string(), None, |ui, _| language_menu(ui, app));
    });
    row.group(RibbonGroup::new("review.comments", tl!("Comments").to_string(), IconId::new("newComment").into()), |ui| {
        Button::command(CommandRef::new("review.newComment"))
            .label(tl!("New\nComment").to_string())
            .icon(IconId::new("newComment"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("review.deleteComment"))
                .label(tl!("Delete").to_string())
                .tooltip(tl!("Delete Comment").to_string())
                .icon(IconId::new("deleteComment"))
                .show(ui);
            Button::command(CommandRef::new("review.previousComment"))
                .label(tl!("Previous").to_string())
                .tooltip(tl!("Previous Comment").to_string())
                .icon(IconId::new("prevComment"))
                .show(ui);
            Button::command(CommandRef::new("review.nextComment"))
                .label(tl!("Next").to_string())
                .tooltip(tl!("Next Comment").to_string())
                .icon(IconId::new("nextComment"))
                .show(ui);
        });
        let shown = app.session.view.comments_pane;
        Button::command(CommandRef::with_params("view.commentsPane", json!({"value": !shown})))
            .label(tl!("Show\nComments").to_string())
            .icon(IconId::new("showComments"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(
        RibbonGroup::new("review.tracking", tl!("Tracking").to_string(), IconId::new("trackChanges").into()).launcher("review.trackingOptions"),
        |ui| {
            ToggleButton::command(CommandRef::new("review.trackChanges"))
                .label(tl!("Track\nChanges").to_string())
                .icon(IconId::new("trackChanges"))
                .arrangement(Arrangement::Stacked)
                .selected(app.session.doc.settings.track_changes)
                .show(ui);
            let all = app.session.view.show_markup;
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                let face = Button::new(if all { tl!("All Markup").to_string() } else { tl!("No Markup").to_string() }).icon(IconId::new("markup"));
                menu_button(ui, face, tl!("Display for Review").to_string(), Some("review.markup"), |ui, _| {
                    MenuItem::command(CommandRef::with_params("review.markup", json!({"value": "all"})))
                        .label(tl!("All Markup").to_string())
                        .radio_mark(all)
                        .show(ui);
                    MenuItem::command(CommandRef::with_params("review.markup", json!({"value": "noMarkup"})))
                        .label(tl!("No Markup").to_string())
                        .radio_mark(!all)
                        .show(ui);
                });
                Button::command(CommandRef::new("review.changes"))
                    .label(tl!("Reviewing Pane").to_string())
                    .icon(IconId::new("reviewingPane"))
                    .show(ui);
            });
        },
    );
    row.group(RibbonGroup::new("review.changes", tl!("Changes").to_string(), IconId::new("accept").into()), |ui| {
        SplitButton::<u8>::command(
            CommandRef::new("review.accept"),
            SplitMenu::Body(Box::new(|ui, _| {
                MenuItem::command(CommandRef::with_params("review.accept", json!({"next": true})))
                    .label(tl!("Accept and Move to Next").to_string())
                    .show(ui);
                MenuItem::command(CommandRef::with_params("review.accept", json!({}))).label(tl!("Accept This Change").to_string()).show(ui);
                MenuItem::command(CommandRef::new("review.acceptAll")).label(tl!("Accept All Changes").to_string()).show(ui);
                MenuItem::command(CommandRef::with_params("review.acceptAll", json!({"stopTracking": true})))
                    .label(tl!("Accept All Changes and Stop Tracking").to_string())
                    .show(ui);
            })),
        )
        .label(tl!("Accept").to_string())
        .icon(IconId::new("accept"))
        .arrangement(Arrangement::Stacked)
        .show(ui);
        SplitButton::<u8>::command(
            CommandRef::new("review.reject"),
            SplitMenu::Body(Box::new(|ui, _| {
                MenuItem::command(CommandRef::with_params("review.reject", json!({"next": true})))
                    .label(tl!("Reject and Move to Next").to_string())
                    .show(ui);
                MenuItem::command(CommandRef::with_params("review.reject", json!({}))).label(tl!("Reject Change").to_string()).show(ui);
                MenuItem::command(CommandRef::new("review.rejectAll")).label(tl!("Reject All Changes").to_string()).show(ui);
                MenuItem::command(CommandRef::with_params("review.rejectAll", json!({"stopTracking": true})))
                    .label(tl!("Reject All Changes and Stop Tracking").to_string())
                    .show(ui);
            })),
        )
        .label(tl!("Reject").to_string())
        .icon(IconId::new("reject"))
        .arrangement(Arrangement::Stacked)
        .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("review.previousChange"))
                .label(tl!("Previous").to_string())
                .tooltip(tl!("Previous Change").to_string())
                .icon(IconId::new("prevChange"))
                .show(ui);
            Button::command(CommandRef::new("review.nextChange"))
                .label(tl!("Next").to_string())
                .tooltip(tl!("Next Change").to_string())
                .icon(IconId::new("nextChange"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("review.compare", tl!("Compare").to_string(), IconId::new("compare").into()), |ui| {
        Button::command(CommandRef::new("review.compare"))
            .label(tl!("Compare").to_string())
            .icon(IconId::new("compare"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("review.protect", tl!("Protect").to_string(), IconId::new("restrict").into()), |ui| {
        Button::command(CommandRef::new("review.blockAuthors"))
            .label(tl!("Block\nAuthors").to_string())
            .icon(IconId::new("blockAuthors"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("review.restrict"))
            .label(tl!("Restrict\nEditing").to_string())
            .icon(IconId::new("restrict"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("review.ink", tl!("Ink").to_string(), IconId::new("hideInk").into()), |ui| {
        ToggleButton::command(CommandRef::new("review.hideInk"))
            .label(tl!("Hide\nInk").to_string())
            .icon(IconId::new("hideInk"))
            .arrangement(Arrangement::Stacked)
            .selected(app.session.view.hide_ink)
            .show(ui);
    });
}

/// Developer › Controls: content controls.
pub(super) fn developer(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    let design = app.session.view.design_mode;
    row.group(RibbonGroup::new("developer.controls", tl!("Controls").to_string(), crate::icons::spec("ccRichText")), |ui| {
        let rows: [[(&'static str, &str); 3]; 3] = [
            [("ccRichText", "developer.richText"), ("ccPlainText", "developer.plainText"), ("ccPicture", "developer.picture")],
            [("ccGallery", "developer.buildingBlock"), ("ccCheckBox", "developer.checkBox"), ("ccComboBox", "developer.comboBox")],
            [("ccDropDown", "developer.dropDown"), ("ccDate", "developer.datePicker"), ("ccRepeat", "developer.repeatingSection")],
        ];
        // Three rows of icon-only buttons fill the group's height; each tooltip is its command's label.
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for cells in rows {
                Row::new().show(ui, |ui, _| {
                    for (icon_name, id) in cells {
                        IconButton::command(crate::icons::spec(icon_name), CommandRef::new(id)).show(ui);
                    }
                });
            }
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            ToggleButton::command(CommandRef::new("developer.designMode"))
                .label(tl!("Design Mode").to_string())
                .icon(IconId::new("designMode"))
                .selected(design)
                .show(ui);
            Button::command(CommandRef::new("developer.properties")).label(tl!("Properties").to_string()).icon(IconId::new("properties")).show(ui);
        });
    });
}

/// The Help tab.
pub(super) fn help(row: &mut RibbonRow<'_>, _app: &mut WordApp) {
    row.group(RibbonGroup::new("help.help", tl!("Help").to_string(), crate::icons::spec("help")), |ui| {
        Button::command(CommandRef::with_params("ui.dialog", json!({"name": "about"})))
            .label(tl!("Help").to_string())
            .icon(crate::icons::spec("help"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("ui.checkForUpdates"))
            .label(tl!("Check for\nUpdates…").to_string())
            .icon(IconId::new("update"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("ui.discord"))
            .label(tl!("Community\nDiscord").to_string())
            .icon(IconId::new("discord"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("help.agents", tl!("Agents").to_string(), IconId::new("macros").into()), |ui| {
        Button::command(CommandRef::with_params("ui.dialog", json!({"name": "commands"})))
            .label(tl!("Commands").to_string())
            .icon(IconId::new("macros"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
}

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::ribbon::{RibbonGroup, RibbonRow}; // PLAN: assumed ribbon-w8 adds the craft-menu dependency and passes the `RibbonRow`
use craft::ui::buttons::{Arrangement, Button, IconButton, SplitButton, SplitMenu, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::MenuItem;
use craft::ui::pickers::Select;

use super::common::menu_button;
use super::{Ui, WordApp, json};

/// The View tab's Views, Immersive, Page Movement, Show, Zoom, Dark Mode, Window, Macros and Templates
/// groups.
pub(super) fn view(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    let v = app.session.view.clone();
    let v_source = app.session.md_source.is_some();
    row.group(RibbonGroup::new("view.views", tl!("Views").to_string(), IconId::new("printLayout").into()), |ui| {
        Button::command(CommandRef::new("view.readMode"))
            .label(tl!("Read\nMode").to_string())
            .icon(IconId::new("readMode"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("view.printLayout"))
            .label(tl!("Print\nLayout").to_string())
            .icon(IconId::new("printLayout"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("view.webLayout"))
            .label(tl!("Web\nLayout").to_string())
            .icon(IconId::new("webLayout"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("view.outline")).label(tl!("Outline").to_string()).icon(IconId::new("outline")).show(ui);
            Button::command(CommandRef::new("view.draft")).label(tl!("Draft").to_string()).icon(IconId::new("draft")).show(ui);
            ToggleButton::command(CommandRef::new("view.markdownSource"))
                .label(tl!("Markdown").to_string())
                .tooltip(tl!("Markdown Source").to_string())
                .icon(IconId::new("markdownSource"))
                .selected(v_source)
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("view.immersive", tl!("Immersive").to_string(), IconId::new("immersive").into()), |ui| {
        Button::command(CommandRef::new("view.focus"))
            .label(tl!("Focus").to_string())
            .icon(IconId::new("focus"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("view.immersive"))
            .label(tl!("Immersive\nReader").to_string())
            .icon(IconId::new("immersive"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("view.pageMovement", tl!("Page Movement").to_string(), IconId::new("vertical").into()), |ui| {
        Button::command(CommandRef::new("view.vertical"))
            .label(tl!("Vertical").to_string())
            .icon(IconId::new("vertical"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("view.sideToSide"))
            .label(tl!("Side\nto Side").to_string())
            .icon(IconId::new("sideToSide"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("view.show", tl!("Show").to_string(), IconId::new("wordcraft.ruler").into()), |ui| {
        // Three rows fill the group's height; a fourth check box would run into its label.
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            show_check(ui, app, "Ruler", "view.ruler", v.ruler);
            show_check(ui, app, "Gridlines", "view.gridlines", v.gridlines);
            show_check(ui, app, "Navigation Pane", "view.navigationPane", v.nav_pane);
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            show_check(ui, app, "Hide White Space", "view.hideWhiteSpace", v.hide_white_space);
        });
    });
    row.group(RibbonGroup::new("view.zoom", tl!("Zoom").to_string(), IconId::new("zoom").into()), |ui| {
        Button::command(CommandRef::with_params("ui.dialog", json!({"name": "zoom"})))
            .label(tl!("Zoom").to_string())
            .icon(IconId::new("zoom"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("view.zoom100"))
            .label(tl!("100%").to_string())
            .icon(IconId::new("zoom100"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        // Step the zoom up and down by 10% (issue #67), from whatever the page shows now.
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("view.zoomIn")).label(tl!("Zoom In").to_string()).icon(IconId::new("zoomIn")).show(ui);
            Button::command(CommandRef::new("view.zoomOut")).label(tl!("Zoom Out").to_string()).icon(IconId::new("zoomOut")).show(ui);
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            ToggleButton::command(CommandRef::new("view.onePage"))
                .label(tl!("One Page").to_string())
                .icon(IconId::new("onePage"))
                .selected(v.fit == "onePage")
                .show(ui);
            ToggleButton::command(CommandRef::new("view.multiplePages"))
                .label(tl!("Multiple Pages").to_string())
                .icon(IconId::new("multiplePages"))
                .selected(v.multi_page)
                .show(ui);
            ToggleButton::command(CommandRef::new("view.pageWidth"))
                .label(tl!("Page Width").to_string())
                .icon(IconId::new("pageWidth"))
                .selected(v.fit == "pageWidth")
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("view.darkMode", tl!("Dark Mode").to_string(), IconId::new("darkMode").into()), |ui| {
        Button::command(CommandRef::new("view.darkMode"))
            .label(tl!("Switch\nModes").to_string())
            .icon(IconId::new("darkMode"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        let face = Button::new(tl!("Interface\nTheme").to_string()).icon(IconId::new("interfaceTheme")).arrangement(Arrangement::Stacked);
        menu_button(ui, face, tl!("Interface Theme").to_string(), None, |ui, _| {
            for a in crate::theme::Appearance::ALL {
                MenuItem::command(CommandRef::with_params("view.interfaceTheme", json!({"value": a.code()})))
                    .label(tl!(a.label()).to_string())
                    .radio_mark(app.ui.theme == a)
                    .show(ui);
            }
        });
    });
    row.group(RibbonGroup::new("view.window", tl!("Window").to_string(), IconId::new("wordcraft.more").into()), |ui| {
        Button::command(CommandRef::new("view.newWindow"))
            .label(tl!("New\nWindow").to_string())
            .icon(IconId::new("newWindow"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("view.arrangeAll"))
            .label(tl!("Arrange\nAll").to_string())
            .icon(IconId::new("arrangeAll"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("view.split"))
            .label(tl!("Split").to_string())
            .icon(IconId::new("wordcraft.split"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        // Across the app's windows (#322; `windows`), read in this pass.
        let w = app.session.windows.clone();
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            if w.side_by_side.is_none() && w.others.len() > 1 {
                SplitButton::<u8>::command(
                    CommandRef::new("view.sideBySide"),
                    SplitMenu::Body(Box::new(|ui, _| window_list(ui, &w.others, "view.sideBySide"))),
                )
                .label(tl!("View Side by Side").to_string())
                .icon(IconId::new("sideBySide"))
                .show(ui);
            } else {
                ToggleButton::command(CommandRef::new("view.sideBySide"))
                    .label(tl!("View Side by Side").to_string())
                    .icon(IconId::new("sideBySide"))
                    .selected(w.side_by_side.is_some())
                    .show(ui);
            }
            ToggleButton::command(CommandRef::new("view.syncScroll"))
                .label(tl!("Synchronous Scrolling").to_string())
                .icon(IconId::new("syncScroll"))
                .selected(w.sync_scroll)
                .show(ui);
        });
        // PLAN: assumed `switch_windows_menu(ui: &mut Ui, app: &mut WordApp)` draws `MenuItem` rows (shell-w8).
        SplitButton::<u8>::command(
            CommandRef::new("view.switchWindows"),
            SplitMenu::Body(Box::new(|ui, _| crate::documents::switch_windows_menu(ui, app))),
        )
        .label(tl!("Switch\nWindows").to_string())
        .icon(IconId::new("switchWindows"))
        .arrangement(Arrangement::Stacked)
        .show(ui);
        ToggleButton::command(CommandRef::new("view.fullScreen"))
            .label(tl!("Full\nScreen").to_string())
            .icon(IconId::new("fullScreen"))
            .arrangement(Arrangement::Stacked)
            .selected(v.full_screen)
            .show(ui);
    });
    row.group(RibbonGroup::new("view.macros", tl!("Macros").to_string(), IconId::new("macros").into()), |ui| {
        Button::command(CommandRef::new("tools.macros"))
            .label(tl!("Macros").to_string())
            .icon(IconId::new("macros"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("view.templates", tl!("Templates").to_string(), IconId::new("documentTemplate").into()), |ui| {
        Button::command(CommandRef::new("tools.templates"))
            .label(tl!("Templates").to_string())
            .icon(IconId::new("documentTemplate"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
}

/// A check box: a change runs `id` with the new value.
fn show_check(ui: &mut Ui, app: &mut WordApp, label: &str, id: &str, on: bool) {
    let out = Checkbox::new(Bind::given(on)).label(tl!(label).to_string()).show(ui);
    if let Some(&value) = out.events.committed() {
        let _ = app.run(id, json!({"value": value}));
    }
}

/// The Outlining tab (Outline view): outline levels, moving, expanding and collapsing, what is
/// shown, and closing the view.
pub(super) fn outlining(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    use wordcraft_engine::layout::outline::{BODY, para_level};
    let o = app.session.view.outline.clone();
    let level = app.session.doc.para_at(&app.session.sel.focus).map(|p| para_level(&app.session.doc, p)).unwrap_or(BODY);
    let name = |l: u8| if l >= BODY { tl!("Body Text").to_string() } else { crate::i18n::fmt(tl!("Level {n}"), &[("n", &l.to_string())]) };
    let levels: Vec<Entry<u8>> = (1..=BODY).map(|l| Entry::new(l, name(l))).collect();
    let mut shown: Vec<Entry<u8>> = vec![Entry::new(0u8, tl!("All Levels").to_string())];
    shown.extend((1..BODY).map(|l| Entry::new(l, name(l))));
    row.group(RibbonGroup::new("outlining.tools", tl!("Outline Tools").to_string(), IconId::new("outlinePromote").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                IconButton::command(IconId::new("outlineTop"), CommandRef::new("outline.promoteToHeading1")).show(ui);
                IconButton::command(IconId::new("outlinePromote"), CommandRef::new("outline.promote")).show(ui);
                let pick = Select::new(tl!("Outline Level").to_string(), Bind::given(level), &levels).show(ui);
                if let Some(&l) = pick.events.committed() {
                    let l = if l >= BODY { json!("body") } else { json!(l) };
                    let _ = app.run("outline.level", json!({ "level": l }));
                }
                IconButton::command(IconId::new("outlineDemote"), CommandRef::new("outline.demote")).show(ui);
                IconButton::command(IconId::new("outlineBody"), CommandRef::new("outline.demoteToBody")).show(ui);
            });
            Row::new().show(ui, |ui, _| {
                IconButton::command(IconId::new("outlineUp"), CommandRef::new("outline.moveUp")).show(ui);
                IconButton::command(IconId::new("outlineDown"), CommandRef::new("outline.moveDown")).show(ui);
                IconButton::command(IconId::new("outlineExpand"), CommandRef::new("outline.expand")).show(ui);
                IconButton::command(IconId::new("outlineCollapse"), CommandRef::new("outline.collapse")).show(ui);
                let pick = Select::new(tl!("Show Level").to_string(), Bind::given(o.show_level), &shown).show(ui);
                if let Some(&l) = pick.events.committed() {
                    let l = if l == 0 { json!("all") } else { json!(l) };
                    let _ = app.run("outline.showLevel", json!({ "level": l }));
                }
            });
            Row::new().show(ui, |ui, _| {
                show_check(ui, app, "Show Text Formatting", "outline.showFormatting", !o.hide_formatting);
                show_check(ui, app, "Show First Line Only", "outline.firstLineOnly", o.first_line_only);
            });
        });
    });
    row.group(RibbonGroup::new("outlining.close", tl!("Close").to_string(), IconId::new("closeOutline").into()), |ui| {
        Button::command(CommandRef::new("outline.close"))
            .label(tl!("Close\nOutline View").to_string())
            .icon(IconId::new("closeOutline"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
}

/// The other WordCraft windows by title; picking one runs `id` with it.
fn window_list(ui: &mut Ui, others: &[wordcraft_engine::cmd::view::OtherWindow], id: &str) {
    if others.is_empty() {
        MenuItem::new(tl!("No other WordCraft windows are open.").to_string()).enabled(false).show(ui);
    }
    for o in others {
        MenuItem::command(CommandRef::with_params(id, json!({"window": o.id}))).label(o.title.clone()).show(ui);
    }
}

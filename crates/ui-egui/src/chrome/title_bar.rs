//! The title bar: AutoSave, the Quick Access Toolbar, the document title, search, community and account.

use std::cell::RefCell;

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::RibbonMenuRequest;
use craft::review::ui::Avatar;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Switch;
use craft::ui::contract::{Bind, BodyCx, Props as _, Show as _, ShowCollection as _, ShowContainer as _, TooltipSpec, Trigger};
use craft::ui::display::{Divider, Text};
use craft::ui::list::{UndoList, UndoMark, UndoRows, UndoWords, footer_text, undo_count};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::popover::Popover;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::inspect;
use craft::ui::runtime::windows::{WindowAction, WindowHost};
use craft::ui::shell::{Title, TitleBar};
use egui::Ui;
use serde_json::json;

use super::{QAT_MENU, qat_enabled, undo_list_rows};
use crate::WordApp;

/// Fills the shell's title slot: the title bar, and what its title and Close report.
pub(crate) fn show(ui: &mut Ui, _cx: &mut BodyCx<'_>, app: &mut WordApp) {
    let title = if app.has_document() {
        let state = if app.session.dirty {
            tl!("Edited")
        } else if app.session.path.is_some() {
            tl!("Saved")
        } else {
            tl!("Not saved")
        };
        Title::new(format!("{} — {state}", craft::i18n::file_name(&app.title_stem(), super::interface_direction())))
            .action(tl!("Rename Document").to_string())
    } else {
        // No document open (#1129): the app's name alone.
        Title::new("WordCraft")
    };
    // The bar runs the leading closure and then the trailing one, never one inside the other
    // (`TitleBar::show_titled`), so each borrow below succeeds.
    let app = RefCell::new(app);
    // The bar always names the document, under the OS's own title bar too (prod `chrome.rs:425-471`).
    let out = TitleBar::new().title_when_native().show_titled(
        ui,
        title,
        |ui, _| {
            if let Ok(mut a) = app.try_borrow_mut() {
                leading(&mut a, ui);
            }
        },
        |ui, _| {
            if let Ok(mut a) = app.try_borrow_mut() {
                trailing(&mut a, ui);
            }
        },
    );
    if out.title_activated
        && let Ok(mut a) = app.try_borrow_mut()
    {
        let _ = a.run("file.rename", json!({}));
    }
    if out.close_requested {
        // The bar never closes the window: the close goes to the OS as its own close button's does, so it passes
        // the unsaved-documents guard.
        let ctx = ui.ctx().clone();
        let _ = WindowHost::of(&ctx).act(ctx.viewport_id(), WindowAction::Close);
    }
}

/// AutoSave and the Quick Access Toolbar, at the start of the bar.
fn leading(app: &mut WordApp, ui: &mut Ui) {
    autosave(app, ui);
    // Hidden (#1945): neither the buttons nor the … menu; the title centres as before.
    if app.ui.quick_access_toolbar {
        quick_access(app, ui);
    }
}

/// The AutoSave switch: off and with the reason as its tooltip when AutoSave can't cover the document (#196,
/// #176); a click then offers Save As where that helps.
fn autosave(app: &mut WordApp, ui: &mut Ui) {
    let block = app.autosave_block();
    let on = app.autosaves() && block.is_none();
    let tip = block.as_ref().map_or_else(|| tl!("AutoSave saves every change to the file (needs a saved document)").to_string(), |b| b.reason());
    let out = Switch::new(Bind::given(on)).label(tl!("AutoSave").to_string()).tooltip(TooltipSpec::new(tip)).show(ui);
    if out.response.clicked() {
        match block {
            None => {
                let _ = app.run("file.autosave", json!({}));
            }
            // AutoSave comes on once Save As has saved (maybe on a later frame, #94).
            Some(b) if b.save_as_helps() => {
                let _ = app.save_as(crate::file_dialogs::AfterSave::AutoSave);
            }
            Some(b) => app.status(b.reason()),
        }
    }
}

/// The user's Quick Access Toolbar commands, then the menu button that customizes it (#376).
fn quick_access(app: &mut WordApp, ui: &mut Ui) {
    let ids: Vec<String> = app.session.ribbon.qat().into_iter().map(str::to_string).collect();
    let ctx = ui.ctx().clone();
    // The buttons are an inspected region, so the keytips badge them.
    inspect::record(ui, crate::keytips::qat_region(), |ui| {
        for id in &ids {
            let enabled = qat_enabled(&ctx, app, id);
            let mut button = IconButton::command(crate::icons::for_command(id), CommandRef::new(id.as_str())).enabled(enabled);
            if id == "edit.undo" {
                button = button.tooltip(TooltipSpec::new(crate::i18n::prefixed("Undo", app.session.undo_label().unwrap_or(""))));
            }
            let out = button.show(ui);
            // The ribbon's command menu: on or off the toolbar, and the customize rows.
            let request =
                RibbonMenuRequest { command: Some(CommandRef::new(id.as_str())), at: out.response.rect.left_bottom() };
            Menu::context(tl!("Quick Access Toolbar").to_string(), &out.response)
                .show(ui, |ui, _| crate::ribbon::command_menu(ui, app, &request));
            if id == "edit.undo" {
                // The list is the document's steps, not the Markdown Source pane's typing.
                undo_arrow(app, ui, enabled && app.session.md_source.is_none());
            }
        }
    });
    qat_menu(app, ui);
}

/// How many steps the undo list shows before it scrolls.
const UNDO_ROWS_SHOWN: u16 = 8;

/// The small arrow beside Undo: a list of steps to undo several at once, with a footer that names how many a pick
/// undoes, or Cancel while none is marked (#1903).
fn undo_arrow(app: &mut WordApp, ui: &mut Ui, enabled: bool) {
    let out = IconButton::new(IconId::new("chevron_down")).label(tl!("Undo History").to_string()).enabled(enabled).show(ui);
    let labels = undo_list_rows(app);
    let mut open = app.chrome.undo;
    if out.response.clicked() {
        open = !open;
    }
    let mut mark = app.chrome.undo_mark;
    let mut picked = None;
    Popover::new(tl!("Undo History").to_string(), Bind::bound(&mut open)).opened_by(&out.response).show(ui, |ui, cx| {
        let rows = UndoRows::new(&labels);
        // A collection owns its scroll: the popover's body scrolls, so the list caps its rows and scrolls the rest.
        let events = UndoList::new(tl!("Undo History").to_string())
            .max_rows(UNDO_ROWS_SHOWN)
            .selection(Bind::bound(&mut mark))
            .show_items(ui, &rows, &mut |key, _| rows.row(*key))
            .events;
        if let Some(count) = undo_count(&events) {
            picked = Some(count);
            cx.request_close();
        }
        Divider::horizontal().show(ui);
        let env = Env::of(ui);
        let words = env.words::<UndoWords>();
        match footer_text(&words, env.format(), &mark) {
            Some(text) => {
                Text::new(text).show(ui);
            }
            None => {
                if Button::new(words.cancel.clone()).show(ui).clicked() {
                    cx.request_close();
                }
            }
        }
    });
    app.chrome.undo = open;
    app.chrome.undo_mark = if open { mark } else { UndoMark::default() };
    if let Some(count) = picked {
        let _ = app.run("edit.undoTo", json!({"count": count}));
    }
}

/// The menu button (…): common commands to show or hide, More Commands…, and Hide.
fn qat_menu(app: &mut WordApp, ui: &mut Ui) {
    let out = IconButton::new(IconId::new("more")).label(tl!("Customize Quick Access Toolbar").to_string()).show(ui);
    let shown: Vec<(&'static str, String, bool)> = QAT_MENU
        .iter()
        .filter_map(|id| {
            let label = app.session.registry.get(id).map(|s| tl!(s.label).to_string())?;
            Some((*id, label, app.session.ribbon.qat().contains(id)))
        })
        .collect();
    // No `open` binding: with one the framework leaves the opening to the app, without it the opener opens the menu.
    Menu::new(tl!("Customize Quick Access Toolbar").to_string())
        .opened_by(&out.response, Trigger::Click)
        .show(ui, |ui, _| {
            MenuItem::heading(tl!("Customize Quick Access Toolbar").to_string()).show(ui);
            for (id, label, on) in &shown {
                let key = if *on { "qatRemove" } else { "qatAdd" };
                MenuItem::command(CommandRef::with_params("tools.customizeRibbon", json!({ key: *id }))).label(label.clone()).check(*on).show(ui);
            }
            Divider::horizontal().show(ui);
            MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "customizeQuickAccess"})))
                .label(tl!("More Commands…").to_string())
                .show(ui);
            // Last, as in Word (#1945); the ribbon's right-click menu brings the toolbar back.
            Divider::horizontal().show(ui);
            MenuItem::command(CommandRef::with_params("ui.quickAccessToolbar", json!({"value": false})))
                .label(tl!("Hide Quick Access Toolbar").to_string())
                .show(ui);
        });
}

/// Search, the community button and the account, at the end of the bar.
fn trailing(app: &mut WordApp, ui: &mut Ui) {
    super::search::show(app, ui);
    if app.ui.show_discord {
        let out = Button::new(tl!("Discord").to_string())
            .icon(IconId::new("discord"))
            .tooltip(TooltipSpec::new(tl!("Join the ArtCraft community on Discord").to_string()))
            .show(ui);
        if out.clicked() {
            let _ = app.run("ui.discord", json!({}));
        }
    }
    let initials: String = app.session.user_initials().chars().take(2).collect();
    let out = Avatar::new(app.session.author.clone())
        .initials(initials)
        .pressable(true)
        .tooltip(TooltipSpec::new(format!("{} — {}", app.session.author, tl!("set your name in File › Options"))))
        .show(ui);
    if out.clicked {
        let _ = app.run("ui.backstage", json!({"value": true, "page": "options", "pane": "general"}));
    }
}

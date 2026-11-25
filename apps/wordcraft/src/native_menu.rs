//! The macOS application menu uses the same About dialog as File › About.

use craft::menu::{CommandRef, Gate, Item, ItemRole, Layout, LayoutOptions, Menu, MenuBar, MenuRole, NativeMenu, mac_layout};
use serde_json::json;
use wordcraft_ui_egui::{WordApp, i18n};

pub(super) fn layout(lang: i18n::Lang) -> Layout {
    let about = Item::new(CommandRef::with_params("ui.dialog", json!({"name": "about"})), i18n::tr(lang, "About WordCraft")).role(ItemRole::About);
    let bar = MenuBar::new(vec![Menu::new("WordCraft", vec![about.into()]).role(MenuRole::App)]);
    let label = |s: &'static str| i18n::tr(lang, s).to_owned();
    let mut layout = mac_layout(&bar, &LayoutOptions::new("WordCraft").label(&label));
    // WordCraft's document menus live in its ribbon; keep only the application menu.
    layout.bar.menus.retain(|menu| menu.role == MenuRole::App);
    layout
}

pub(super) fn install(ctx: &egui::Context, app: &WordApp) -> Option<NativeMenu> {
    let menu = craft::menu::mac::install(ctx, &layout(i18n::Lang::from_pref(&app.ui.language)).bar);
    if menu.is_none() {
        log::warn!("the WordCraft application menu could not be installed");
    }
    menu
}

pub(super) fn update(menu: &mut NativeMenu, app: &mut WordApp, ctx: &egui::Context) {
    menu.set_gate(Gate::default().modal(app.dialog_host_mut().shows_modal()));
    for command in menu.take_clicks(ctx) {
        if let Err(error) = app.run(&command.id, command.params) {
            log::warn!("application menu: {error}");
        }
    }
    menu.set_gate(Gate::default().modal(app.dialog_host_mut().shows_modal()));
    let lang = i18n::Lang::from_pref(&app.ui.language);
    for clash in menu.sync_with(lang.code(), || layout(lang)) {
        log::warn!("application menu: {clash:?}");
    }
}

//! The macOS application menu uses the same About dialog as File › About.

use craft::menu::{CommandRef, Gate, Item, ItemRole, Layout, LayoutOptions, Menu, MenuBar, MenuRole, NativeMenu, mac_layout};
use serde_json::json;
use wordcraft_ui_egui::{WordApp, i18n};

const APP_NAME: &str = "WordCraft";
const ABOUT: &str = "About WordCraft";

/// The application menu, with `tr` the translation of an English string.
pub(super) fn layout(tr: &dyn Fn(&'static str) -> String) -> Layout {
    let about = Item::new(CommandRef::with_params("ui.dialog", json!({"name": "about"})), tr(ABOUT)).role(ItemRole::About);
    let bar = MenuBar::new(vec![Menu::new(APP_NAME, vec![about.into()]).role(MenuRole::App)]);
    // `mac_layout` words About as the template; WordCraft's catalogs translate the whole label.
    let label = |s: &'static str| tr(if s == "About {app}" { ABOUT } else { s });
    let mut layout = mac_layout(&bar, &LayoutOptions::new(APP_NAME).label(&label));
    // WordCraft's document menus live in its ribbon; keep only the application menu.
    layout.bar.menus.retain(|menu| menu.role == MenuRole::App);
    layout
}

fn lang(app: &WordApp) -> i18n::Lang {
    i18n::Lang::from_pref(&app.ui.language)
}

fn layout_in(lang: i18n::Lang) -> Layout {
    layout(&|s| i18n::tr(lang, s).to_owned())
}

pub(super) fn install(ctx: &egui::Context, app: &WordApp) -> Option<NativeMenu> {
    let menu = craft::menu::mac::install(ctx, &layout_in(lang(app)).bar);
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
    let lang = lang(app);
    for clash in menu.sync_with(lang.code(), || layout_in(lang)) {
        log::warn!("application menu: {clash:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalogs translate "About WordCraft" and have no "About {app}" row.
    #[test]
    fn about_shows_the_catalogs_translation() {
        let translate = |s: &'static str| if s == "About WordCraft" { "Über WordCraft".to_owned() } else { s.to_owned() };
        let layout = layout(&translate);
        let about = layout.bar.items().into_iter().find(|item| item.role == Some(ItemRole::About)).unwrap();
        assert_eq!(about.label, "Über WordCraft");
    }
}

//! The ribbon's right-click menu rows: Quick Access Toolbar membership and the two Customize dialogs.

use craft::keymap::CommandRef;
use craft::menu::RibbonMenuRequest;
use craft::ui::contract::{Props as _, Show as _};
use craft::ui::display::Divider;
use craft::ui::menu::MenuItem;
use egui::Ui;
use serde_json::{Value, json};

use crate::WordApp;

/// One row bound to the command `id` run with `params`, labelled `label`.
fn row(ui: &mut Ui, label: &str, id: &str, params: Value) {
    MenuItem::command(CommandRef::with_params(id, params)).label(label.to_string()).show(ui);
}

/// The rows of a ribbon control's context menu: put its command on (or take it off) the Quick Access Toolbar, then the
/// customize rows (#379). `qat` holds the toolbar's command ids and `qat_shown` whether the toolbar is visible. A
/// request with no command gets the customize rows alone.
pub(crate) fn rows(ui: &mut Ui, req: &RibbonMenuRequest, qat: &[String], qat_shown: bool) {
    if let Some(command) = &req.command {
        let id = command.id.as_str();
        if qat.iter().any(|q| q == id) {
            row(ui, tl!("Remove from Quick Access Toolbar"), "tools.customizeRibbon", json!({"qatRemove": id}));
        } else {
            row(ui, tl!("Add to Quick Access Toolbar"), "tools.customizeRibbon", json!({"qatAdd": id}));
        }
        Divider::horizontal().show(ui);
    }
    customize_rows(ui, qat_shown);
}

/// The rows of a command's context menu from the app's state: the title bar's Quick Access Toolbar uses it.
pub(crate) fn command_menu(ui: &mut Ui, app: &mut WordApp, req: &RibbonMenuRequest) {
    let qat: Vec<String> = app.session.ribbon.qat().into_iter().map(str::to_string).collect();
    rows(ui, req, &qat, app.ui.quick_access_toolbar);
}

fn customize_rows(ui: &mut Ui, qat_shown: bool) {
    if !qat_shown {
        row(ui, tl!("Show Quick Access Toolbar"), "ui.quickAccessToolbar", json!({"value": true}));
    }
    row(ui, tl!("Customize the Ribbon…"), "ui.dialog", json!({"name": "customizeRibbon"}));
    row(ui, tl!("Customize Quick Access Toolbar…"), "ui.dialog", json!({"name": "customizeQuickAccess"}));
}

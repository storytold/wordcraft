//! The slim bar Focus and Immersive show in place of the chrome, filled into the shell's screen-bar slot.

use craft::behavior::IconId;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{BodyCx, Props as _, Show as _, TooltipSpec};
use egui::Ui;
use serde_json::json;

use super::Screen;
use crate::WordApp;

/// Fills the screen-bar slot. Focus: a `more` button that drops the ribbon down over the page. Immersive: the
/// command search field.
pub(crate) fn show(ui: &mut Ui, _cx: &mut BodyCx<'_>, app: &mut WordApp) {
    match Screen::of(&app.session.view) {
        Screen::Focus => {
            let out = IconButton::new(IconId::new("wordcraft.more"))
                .label(tl!("Show the ribbon").to_string())
                .tooltip(TooltipSpec::new(tl!("Show the ribbon").to_string()))
                .show(ui);
            if out.response.clicked() {
                let _ = app.run("ui.focusRibbon", json!({}));
            }
        }
        // The slim bar holds the field alone, so it always has its room.
        _ => {
            super::search::show(app, ui, false);
        }
    }
}

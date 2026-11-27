//! The page area with no document open (#3126).

use craft::ui::buttons::Button;
use craft::ui::contract::{Props as _, Show as _, clicked};
use craft::ui::display::{Shade, Text};
use egui::Ui;

use crate::WordApp;

use super::json;

/// The page area with no document open (#3126): what to do next, so Esc on the Start screen or
/// File › Close leaves the ways to make or open a document in sight instead of just a greyed-out
/// ribbon that looks broken. The page itself stays empty until one is made or opened, as #1129
/// has it; Home goes back to the Start screen.
pub(crate) fn empty_state(app: &mut WordApp, ui: &mut Ui) {
    let full = ui.available_rect_before_wrap();
    // Centred in the page area, but never pushed off the top of a short window.
    ui.add_space(((full.height() - 100.0) * 0.5).max(24.0));
    ui.vertical_centered(|ui| {
        Text::new(tl!("No document is open. Create a document or open one first.")).shade(Shade::Muted).id_salt("empty-copy").show(ui);
        ui.add_space(16.0);
        if clicked(&Button::new(tl!("Blank document")).id_salt("empty-blank").show(ui)) {
            let _ = app.run("file.new", json!({"template": "blank"}));
            app.canvas.focus_page_after_pass();
        }
        ui.add_space(2.0);
        if clicked(&Button::new(tl!("Browse…")).id_salt("empty-browse").show(ui)) {
            let _ = app.run("ui.openFileDialog", json!({}));
        }
        ui.add_space(8.0);
        // A Quiet button, so it reads as a link beside the two actions; the ribbon's Home tab
        // is a button above it, so the test takes the lower one with the label.
        if clicked(&Button::new(tl!("Home")).id_salt("empty-home").show(ui)) {
            let _ = app.run("ui.backstage", json!({"value": true, "page": "home"}));
        }
    });
}

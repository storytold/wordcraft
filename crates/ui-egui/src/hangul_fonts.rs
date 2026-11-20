//! The Hangul fonts bar in the browser: a page can't reach the system's fonts, and the web build
//! bundles no Korean face, so a document opened from an .hwp or .hwpx shows its Hangul as boxes
//! until the system's fonts are loaded. The bar offers Load Installed Fonts, which brings in the
//! fonts installed on the computer (the browser asks first). Native apps read the system's fonts
//! themselves and never show it.

use egui::{RichText, Ui};

use crate::WordApp;
use crate::theme::Tokens;

/// Whether the bar was closed (or used) for this document, kept in egui's memory.
fn dismissed_id(document: u64) -> egui::Id {
    egui::Id::unique(("hangul_fonts_bar", document))
}

/// The bar under the ribbon, for a document opened from a Hangul file in the browser.
pub(crate) fn bar(app: &mut WordApp, ui: &mut Ui) {
    if app.services.load_system_fonts.is_none() {
        return;
    }
    let passthrough = &app.session.doc.passthrough;
    if !passthrough.contains_key(wordcraft_engine::io::HWPX_SOURCE) && !passthrough.contains_key(wordcraft_engine::io::HWP_SOURCE) {
        return;
    }
    let id = dismissed_id(app.session.document_id());
    if ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    let message = tl!("Korean text shows as boxes until fonts are loaded: the browser can't use the fonts installed on this computer.");
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("hangul_fonts_bar")
        .exact_size(36.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(12, 0)))
        .show(ui, |ui| {
            let full = ui.max_rect();
            ui.painter().hline(full.x_range(), full.bottom() - 0.5, egui::Stroke::new(1.0, t.border));
            ui.horizontal_centered(|ui| {
                ui.add(egui::Label::new(RichText::new(message).color(t.text)).truncate());
                if ui.add(crate::widgets::primary_button(ui, tl!("Load Installed Fonts"))).clicked() {
                    app.load_system_fonts();
                    ui.ctx().data_mut(|d| d.insert_temp(id, true));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::icon_button(ui, "close", tl!("Not now")).clicked() {
                        ui.ctx().data_mut(|d| d.insert_temp(id, true));
                    }
                });
            });
        });
}

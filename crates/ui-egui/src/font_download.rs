//! "Get the missing fonts": a document opened from a file that asks for fonts their publisher
//! gives away (Microsoft's Aptos, the default in recent Word documents) is told about once, in a
//! box that names each font, the font drawn in its place, the publisher's download page and how
//! to install the download. "Don't show this again" turns the box off (File › Options › General
//! turns it back on); the status bar's missing-fonts menu opens it by hand.
//!
//! The list is `font.missing`'s ([`wordcraft_engine::cmd::font::missing_fonts`]), so agents read
//! the same fonts and pages without the box. WordCraft only links to the page: it never
//! downloads or bundles the fonts.

use serde::Serialize;

use crate::WordApp;
use crate::dialogs::Dialog;

/// Most documents the box has been considered for that are remembered: beyond that the oldest
/// are forgotten (a document shown again after hundreds of others may be told about twice).
const REMEMBERED: usize = 256;

/// Most fonts the box lists: a hostile file can name thousands.
const LISTED: usize = 12;

/// One missing font the box lists.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// The font the document asks for.
    pub font: String,
    /// The families its text is drawn in instead.
    pub shown_as: Vec<String>,
    /// Who publishes the font and hosts the page.
    pub publisher: String,
    /// The publisher's download page.
    pub url: String,
}

/// The shown document's missing fonts that can be downloaded.
pub(crate) fn rows(app: &mut WordApp) -> Vec<Row> {
    let layout = app.session.layout();
    let missing = app.cached_missing_fonts(&layout).0;
    missing
        .iter()
        .filter_map(|f| {
            let d = f.download?;
            Some(Row { font: f.font.clone(), shown_as: f.shown_as.clone(), publisher: d.publisher.into(), url: d.url.into() })
        })
        .take(LISTED)
        .collect()
}

/// The box for the shown document (`ui.dialog {"name": "fontDownload"}`), listing nothing when
/// none of its missing fonts can be downloaded.
pub(crate) fn open(app: &mut WordApp) -> Dialog {
    Dialog::FontDownload { fonts: rows(app), dont_show: !app.ui.font_download_notice }
}

/// The document `id` has been considered for the box, and is not considered again.
fn remember(app: &mut WordApp, id: u64) {
    if app.font_notice_seen.len() >= REMEMBERED {
        app.font_notice_seen.remove(0);
    }
    app.font_notice_seen.push(id);
}

/// A document a script opened is not told about: programmatic calls never open dialogs.
pub(crate) fn skip(app: &mut WordApp, id: u64) {
    if !app.font_notice_seen.contains(&id) {
        remember(app, id);
    }
}

impl WordApp {
    /// Bring the box up by itself when a document opened from a file is missing fonts that can be
    /// downloaded. The desktop and web apps call this once at start-up; unit tests and `ui_shot`
    /// don't, so what they show doesn't depend on the fonts installed where they run.
    pub fn tell_about_font_downloads(&mut self) {
        self.font_notices = true;
    }
}

/// Show the box once for a document opened from a file, when it is missing fonts that can be
/// downloaded and the box hasn't been turned off. Called every frame; it waits for an open
/// dialog or the File tab to close rather than replace them.
pub(crate) fn tick(app: &mut WordApp) {
    let id = app.session.document_id();
    if !app.font_notices || app.font_notice_seen.contains(&id) || app.dialog.is_some() || app.ui.backstage {
        return;
    }
    remember(app, id);
    // A new document has no file: its fonts are WordCraft's choice, not a document's from elsewhere.
    if !app.ui.font_download_notice || app.session.path.is_none() {
        return;
    }
    let fonts = rows(app);
    if !fonts.is_empty() {
        app.dialog = Some(Dialog::FontDownload { fonts, dont_show: false });
    }
}

/// How to install a downloaded font on this system, after the files are unpacked.
fn system_install_steps() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        ""
    } else if cfg!(target_os = "windows") {
        tl!("Or install it for every app: select the font files, right-click and choose Install, then restart WordCraft.")
    } else if cfg!(target_os = "macos") {
        tl!("Or install it for every app: double-click each font file and choose Install in Font Book, then restart WordCraft.")
    } else {
        tl!("Or install it for every app: copy the font files to the .local/share/fonts folder in your home folder, then restart WordCraft.")
    }
}

/// The box's content. Returns true to close.
pub(crate) fn body(app: &mut WordApp, ui: &mut egui::Ui, fonts: &[Row], dont_show: &mut bool) -> bool {
    ui.set_width(520.0);
    if fonts.is_empty() {
        ui.label(tl!("None of the fonts this document is missing can be downloaded free of charge from their publisher."));
        return crate::dialogs::close_button(ui);
    }
    ui.label(tl!(
        "This document uses fonts that aren't installed, so it is shown in other fonts and may not look as it was written. These fonts can be downloaded free of charge:"
    ));
    ui.add_space(8.0);
    crate::rtl::Table::new("font_download").columns(3).spacing(egui::vec2(24.0, 6.0)).show(ui, |t| {
        t.row(|r| {
            r.cell(|ui| ui.label(egui::RichText::new(tl!("Font")).strong()));
            r.cell(|ui| ui.label(egui::RichText::new(tl!("Shown as")).strong()));
            r.cell(|ui| ui.label(egui::RichText::new(tl!("Download")).strong()));
        });
        for f in fonts {
            t.row(|r| {
                r.cell(|ui| ui.label(&f.font));
                r.cell(|ui| ui.label(f.shown_as.join(", ")));
                r.cell(|ui| {
                    let label = crate::i18n::fmt(tl!("Download from {publisher}"), &[("publisher", &f.publisher)]);
                    ui.hyperlink_to(label, &f.url).on_hover_text(&f.url)
                });
            });
        }
    });
    ui.add_space(10.0);
    ui.label(egui::RichText::new(tl!("To install a font")).strong());
    ui.label(tl!("1. Open its download page and download the font. If it comes as a .zip file, unpack it."));
    ui.label(tl!("2. Choose Add Font… below and pick the font files. The document is shown in them at once."));
    let system = system_install_steps();
    if !system.is_empty() {
        ui.label(system);
    }
    ui.add_space(10.0);
    if ui.checkbox(dont_show, tl!("Don't show this again")).changed() {
        // Kept at once, however the box is closed.
        app.ui.font_download_notice = !*dont_show;
    }
    ui.add_space(8.0);
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let mut add = false;
    ui.horizontal(|ui| {
        add = ui.button(tl!("Add Font…")).clicked();
        ui.with_layout(crate::rtl::far_side(ui, egui::Align::Center), |ui| {
            close |= ui.add(crate::widgets::primary_button(ui, tl!("Close"))).clicked();
        });
    });
    if add {
        // The file picker replaces the box; the added font lays the document out again.
        let _ = app.run("font.add", serde_json::json!({}));
        return true;
    }
    close
}

#[cfg(test)]
mod tests;

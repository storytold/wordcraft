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
use crate::theme::{Tokens, semibold};

/// Most documents the box has been considered for that are remembered: beyond that the oldest
/// are forgotten (a document shown again after hundreds of others may be told about twice).
const REMEMBERED: usize = 256;

/// Most fonts the box lists: a hostile file can name thousands.
const LISTED: usize = 12;

/// The height the box takes around what scrolls: the title bar, the row with Close and the
/// window's margins.
const ROOM_AROUND_SCROLL: f32 = 190.0;
/// The least height what scrolls gets, however short the window.
const SCROLL_MIN: f32 = 80.0;

/// One missing font the box lists.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// The font the document asks for.
    pub font: String,
    /// The families its text is drawn in instead.
    pub shown_as: Vec<String>,
    /// The download the font comes in (a family: "Aptos" holds Aptos Display too).
    pub download: String,
    /// Who publishes the font and hosts the page.
    pub publisher: String,
    /// The publisher's download page.
    pub url: String,
    /// The publisher limits who may use the font: the box says so beside the download.
    pub restricted: bool,
}

/// The shown document's missing fonts that can be downloaded.
pub(crate) fn rows(app: &mut WordApp) -> Vec<Row> {
    let layout = app.session.layout();
    let missing = app.cached_missing_fonts(&layout).0;
    missing
        .iter()
        .filter_map(|f| {
            let d = f.download?;
            Some(Row {
                font: f.font.clone(),
                shown_as: f.shown_as.clone(),
                download: d.name.clone(),
                publisher: d.publisher.clone(),
                url: d.url.clone(),
                restricted: d.restricted,
            })
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

/// The Close button (Escape too) at the far side, in a row of its own: the far-side layout
/// then takes one row's height and not the rest of the window's.
fn close_row(ui: &mut egui::Ui) -> bool {
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.horizontal(|ui| {
        ui.with_layout(crate::rtl::far_side(ui, egui::Align::Center), |ui| {
            close |= ui.add(crate::widgets::primary_button(ui, tl!("Close"))).clicked();
        });
    });
    close
}

/// The fonts one download brings: listed once, however many of its fonts are missing.
struct Group<'a> {
    /// Its missing fonts.
    fonts: Vec<&'a str>,
    /// The families they are drawn in instead, each once.
    shown_as: Vec<&'a str>,
    publisher: &'a str,
    url: &'a str,
    /// The publisher limits who may use them.
    restricted: bool,
}

/// `fonts` by download page, in the order first met.
fn groups(fonts: &[Row]) -> Vec<Group<'_>> {
    let mut out: Vec<Group> = Vec::new();
    for f in fonts {
        let at = match out.iter().position(|g| g.url == f.url && g.publisher == f.publisher && g.restricted == f.restricted) {
            Some(at) => at,
            None => {
                out.push(Group { fonts: Vec::new(), shown_as: Vec::new(), publisher: &f.publisher, url: &f.url, restricted: f.restricted });
                out.len() - 1
            }
        };
        if let Some(g) = out.get_mut(at) {
            g.fonts.push(&f.font);
            for s in &f.shown_as {
                if !g.shown_as.contains(&s.as_str()) {
                    g.shown_as.push(s);
                }
            }
        }
    }
    out
}

/// A step's number in a filled circle.
fn step_badge(ui: &mut egui::Ui, n: usize, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 10.0, t.accent);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, n.to_string(), semibold(12.0), t.on_accent);
}

/// The box's content: a headline, a card to a download with its button, the two steps that
/// add the downloaded font (Add Font… beside the step that uses it), then the opt-out and Close.
/// Returns true to close.
pub(crate) fn body(app: &mut WordApp, ui: &mut egui::Ui, fonts: &[Row], dont_show: &mut bool) -> bool {
    ui.set_width(520.0);
    if fonts.is_empty() {
        ui.label(tl!("None of the fonts this document is missing can be downloaded free of charge from their publisher."));
        ui.add_space(8.0);
        return close_row(ui);
    }
    let t = Tokens::get(ui.ctx());
    let mut open_url = None;
    let mut add = false;
    // Everything above the row with Close scrolls when the window is too short for it, so
    // that row is on screen and the rest in reach however many downloads there are.
    let room = (ui.ctx().content_rect().height() - ROOM_AROUND_SCROLL).max(SCROLL_MIN);
    egui::ScrollArea::vertical().id_salt("font_downloads").max_height(room).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), r, "fontMissing", t.icon, t.accent);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                // `fonts` holds only the missing fonts that can be downloaded: the document may
                // be missing others, so the headline counts downloads, not missing fonts.
                let headline = crate::i18n::tn(fonts.len() as u64, "{n} missing font can be downloaded", "{n} missing fonts can be downloaded");
                ui.label(egui::RichText::new(headline).font(semibold(15.0)));
                let note =
                    tl!("Text in a missing font is shown in another font, so it may not look as it was written. The downloads are free of charge.");
                ui.label(egui::RichText::new(note).color(t.text_dim));
            });
        });
        ui.add_space(12.0);
        for g in groups(fonts) {
            egui::Frame::NONE
                .fill(t.panel)
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            // A label to a name: each is the document's own word, not interface text.
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                for (i, font) in g.fonts.iter().enumerate() {
                                    if i > 0 {
                                        ui.label(egui::RichText::new(", ").font(semibold(14.0)));
                                    }
                                    ui.label(egui::RichText::new(*font).font(semibold(14.0)));
                                }
                            });
                            let shown = crate::i18n::fmt(
                                tl!("Shown as {fonts} · {publisher}"),
                                &[("fonts", &g.shown_as.join(", ")), ("publisher", g.publisher)],
                            );
                            ui.label(egui::RichText::new(shown).color(t.text_dim));
                            if g.restricted {
                                let limits = tl!("Its licence limits who may use it: read the terms first.");
                                ui.label(egui::RichText::new(limits).small().color(t.text_dim));
                            }
                        });
                        ui.with_layout(crate::rtl::far_side(ui, egui::Align::Center), |ui| {
                            let tip = crate::i18n::fmt(tl!("Download from {publisher}"), &[("publisher", g.publisher)]);
                            if ui.add(crate::widgets::primary_button(ui, tl!("Download"))).on_hover_text(format!("{tip}\n{}", g.url)).clicked() {
                                open_url = Some(g.url.to_string());
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }
        if open_url.is_some() {
            // The publisher's page, in the browser: WordCraft downloads nothing itself.
            app.canvas.open_url = open_url;
        }
        // Whose part the licence is: WordCraft only points at the publisher's page.
        let terms = tl!("Fonts come with licence terms. Read the publisher's terms on the download page: following them is up to you.");
        ui.label(egui::RichText::new(terms).small().color(t.text_dim));
        ui.add_space(12.0);
        ui.label(egui::RichText::new(tl!("Then add it to WordCraft")).font(semibold(14.0)));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            step_badge(ui, 1, &t);
            ui.label(tl!("Download the font. If it comes as a .zip file, unpack it."));
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            step_badge(ui, 2, &t);
            ui.label(tl!("Pick the font files:"));
            add = ui.button(tl!("Add Font…")).clicked();
        });
        let system = system_install_steps();
        if !system.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(system).small().color(t.text_dim));
        }
    });
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);
    // The downloads are the accent buttons; Close (Escape too) is plain beside them.
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.horizontal(|ui| {
        if ui.checkbox(dont_show, tl!("Don't show this again")).changed() {
            // Kept at once, however the box is closed.
            app.ui.font_download_notice = !*dont_show;
        }
        ui.with_layout(crate::rtl::far_side(ui, egui::Align::Center), |ui| {
            close |= ui.button(tl!("Close")).clicked();
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

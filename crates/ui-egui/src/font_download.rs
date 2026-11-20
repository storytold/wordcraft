//! "Get the missing fonts": a document opened from a file that asks for fonts their publisher
//! gives away (Microsoft's Aptos, the default in recent Word documents) is told about once, in a
//! box that names each font, the font drawn in its place, the publisher's download page and how
//! to install the download. "Don't show this again" turns the box off (File › Options › General
//! turns it back on); the status bar's missing-fonts menu opens it by hand.
//!
//! The list is `font.missing`'s ([`wordcraft_engine::cmd::font::missing_fonts`]), so agents read
//! the same fonts and pages without the box. WordCraft only links to the page: it never
//! downloads or bundles the fonts.

use std::cell::Cell;

use craft::behavior::IconId;
use craft::theme::Variant;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Badge, Icon, Shade, Text};
use craft::ui::layout_components::Row as Line;
use craft::ui::primitives::IconSpec;
use craft::ui::sections::{DialogRole, FooterButton, GroupBox};
use egui::Ui;
use serde::Serialize;

use crate::WordApp;
use crate::dialogs::host::{DialogHooks, Settle, open_with};

/// Most documents the box has been considered for that are remembered: beyond that the oldest
/// are forgotten (a document shown again after hundreds of others may be told about twice).
const REMEMBERED: usize = 256;

/// Most fonts the box lists: a hostile file can name thousands.
const LISTED: usize = 12;

/// The key of the box on the dialog host (`ui.dialog {"name": "fontDownload"}`).
const KEY: &str = "fontDownload";

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

/// The dialog name table's entry for the box (`ui.dialog {"name": "fontDownload"}`): opens it for the shown
/// document, listing nothing when none of its missing fonts can be downloaded. `None` for another name.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &serde_json::Value, parent: Option<DialogId>) -> Option<DialogId> {
    if name != KEY {
        return None;
    }
    let font_box = FontBox::new(rows(app), !app.ui.font_download_notice);
    Some(open_with(app, parent, font_box, DialogHooks { prepare: None, settle }))
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
    if !app.font_notices || app.font_notice_seen.contains(&id) || !app.dialog_host.is_empty() || app.ui.backstage {
        return;
    }
    remember(app, id);
    // A new document has no file: its fonts are WordCraft's choice, not a document's from elsewhere.
    if !app.ui.font_download_notice || app.session.path.is_none() {
        return;
    }
    if !rows(app).is_empty() {
        open(app, KEY, &serde_json::Value::Null, None);
    }
}

/// What the box's clicks ask of the app, which the box cannot reach itself; [`settle`] applies them in the pass
/// they were made in.
enum Ask {
    /// Open the publisher's page in the browser: WordCraft downloads nothing itself.
    Download(String),
    /// The opt-out was changed to this; kept at once, however the box is closed.
    OptOut(bool),
}

/// How the box ended.
pub(crate) enum FontBoxResult {
    /// Close, Escape or the title bar's close box.
    Closed,
    /// Add Font… was clicked: the file picker replaces the box and the added font lays the document out again.
    AddFont,
}

/// Applies what the box asked since the last pass and runs Add Font… when the box ended with it.
fn settle(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let taken = app.dialog_host.take_by_id::<FontBox>(id);
    let finished = taken.is_some();
    let added = matches!(taken, Some(DialogResult::Ok(FontBoxResult::AddFont)));
    let asks = app.dialog_host.get_mut_by_id::<FontBox>(id).map(|b| std::mem::take(&mut b.asks)).unwrap_or_default();
    for ask in asks {
        match ask {
            Ask::Download(url) => app.canvas.open_url = Some(url),
            Ask::OptOut(dont_show) => app.ui.font_download_notice = !dont_show,
        }
    }
    if added {
        let _ = app.run("font.add", serde_json::json!({}));
    }
    if finished { Settle::Done } else { Settle::Keep }
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

/// The "Get the Missing Fonts" box on the dialog host: a headline, a card to each download with its button, the
/// two steps that add the downloaded font (Add Font… beside the step that uses it), the opt-out in the footer
/// and Close. The box cannot reach the app, so a click that needs it is recorded in `asks` and [`settle`] applies it
/// in the same pass.
pub(crate) struct FontBox {
    /// The missing fonts that can be downloaded; empty says so in words.
    pub(crate) fonts: Vec<Row>,
    /// "Don't show this again".
    pub(crate) dont_show: bool,
    /// What the clicks of this pass ask of the app.
    asks: Vec<Ask>,
}

impl FontBox {
    /// A box listing `fonts`, with the opt-out as `dont_show`.
    pub(crate) fn new(fonts: Vec<Row>, dont_show: bool) -> Self {
        FontBox { fonts, dont_show, asks: Vec::new() }
    }
}

impl StackDialog for FontBox {
    type Result = FontBoxResult;

    fn id(&self) -> DialogId {
        dialog_id(KEY)
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<FontBoxResult>> {
        let FontBox { fonts, dont_show, asks } = self;
        let opted = Cell::new(None::<bool>);
        let add = Cell::new(false);
        let current = *dont_show;
        let shown = Dialog::new(tl!("Get the Missing Fonts"))
            .footer(|ui, _| {
                let box_state = Checkbox::new(Bind::given(current)).label(tl!("Don't show this again")).id_salt("dont_show").show(ui);
                if let Some(on) = box_state.value().copied() {
                    opted.set(Some(on));
                }
            })
            .buttons([FooterButton::new(tl!("Close"), DialogRole::Cancel)])
            .show(ui, |ui, _| {
                if fonts.is_empty() {
                    Text::new(tl!("None of the fonts this document is missing can be downloaded free of charge from their publisher.")).show(ui);
                    return;
                }
                Line::new().show(ui, |ui, _| {
                    Icon::new(IconSpec::svg(IconId::new("fontMissing"))).show(ui);
                    // `fonts` holds only the missing fonts that can be downloaded: the document may
                    // be missing others, so the headline counts downloads, not missing fonts.
                    Text::new(crate::i18n::tn(fonts.len() as u64, "{n} missing font can be downloaded", "{n} missing fonts can be downloaded"))
                        .show(ui);
                });
                Text::new(tl!(
                    "Text in a missing font is shown in another font, so it may not look as it was written. The downloads are free of charge."
                ))
                .shade(Shade::Faint)
                .show(ui);
                for (at, g) in groups(fonts).into_iter().enumerate() {
                    // The names are the document's own words, not interface text.
                    GroupBox::new(g.fonts.join(", "), None).id_salt(("download", at)).show(ui, |ui, _| {
                        let shown_as =
                            crate::i18n::fmt(tl!("Shown as {fonts} · {publisher}"), &[("fonts", &g.shown_as.join(", ")), ("publisher", g.publisher)]);
                        Text::new(shown_as).shade(Shade::Faint).show(ui);
                        if g.restricted {
                            Text::new(tl!("Its licence limits who may use it: read the terms first.")).shade(Shade::Faint).show(ui);
                        }
                        Line::new().show(ui, |ui, _| {
                            if clicked(&Button::new(tl!("Download")).variant(Variant::PRIMARY).id_salt(("download_button", at)).show(ui)) {
                                asks.push(Ask::Download(g.url.to_string()));
                            }
                        });
                    });
                }
                // Whose part the licence is: WordCraft only points at the publisher's page.
                Text::new(tl!("Fonts come with licence terms. Read the publisher's terms on the download page: following them is up to you."))
                    .shade(Shade::Faint)
                    .show(ui);
                Text::new(tl!("Then add it to WordCraft")).show(ui);
                Line::new().show(ui, |ui, _| {
                    Badge::count(1).show(ui);
                    Text::new(tl!("Download the font. If it comes as a .zip file, unpack it.")).show(ui);
                });
                Line::new().show(ui, |ui, _| {
                    Badge::count(2).show(ui);
                    Text::new(tl!("Pick the font files:")).show(ui);
                    if clicked(&Button::new(tl!("Add Font…")).id_salt("add_font").show(ui)) {
                        add.set(true);
                    }
                });
                let system = system_install_steps();
                if !system.is_empty() {
                    Text::new(system).shade(Shade::Faint).show(ui);
                }
            });
        if let Some(on) = opted.get() {
            *dont_show = on;
            asks.push(Ask::OptOut(on));
        }
        if add.get() {
            return Some(DialogResult::Ok(FontBoxResult::AddFont));
        }
        shown.output.closed.map(|_| DialogResult::Ok(FontBoxResult::Closed))
    }
}

#[cfg(test)]
mod tests;

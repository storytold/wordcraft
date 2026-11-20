//! File › New: a search field, the built-in templates, then the personal templates of the user's
//! templates folder by category.

use std::borrow::Cow;

use craft::behavior::{IconId, keyed};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{
    ActivateOn, Bind, Gap, Props as _, Remember as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use craft::ui::layout_components::Stack;
use craft::ui::list::ThumbAspect;
use craft::ui::notices::Notice;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::Label;
use craft::ui::shell::{Template, ThumbKey};
use craft::ui::text::TextField;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::Ui;
use serde_json::json;

use super::home::template_cards;
use super::{PersonalListing, PersonalTemplate, activated, heading, label, matching_personal, matching_templates, open_personal, subheading, thumbs};
use crate::WordApp;

/// The aspect of a template's page picture (portrait, as a letter page).
const PAGE_WIDE: u16 = 3;
/// The height part of the page picture's aspect.
const PAGE_HIGH: u16 = 4;

/// A grid of template tiles, one per card, that reports the chosen key.
fn grid(name: Label) -> TileGrid<'static, String> {
    TileGrid::<String>::new(name)
        .select(SelectMode::Single)
        .activate(ActivateOn::Click)
        .caption(CaptionPlace::Below)
        .card()
        .aspect(f32::from(PAGE_WIDE) / f32::from(PAGE_HIGH))
        .sizing(Sizing::FillWidth)
        .natural_height(true)
}

/// The user templates folder and its templates (#2852), read from disk. A folder that isn't there
/// lists nothing.
fn read_listing(app: &WordApp) -> PersonalListing {
    let key = (app.session.prefs.user_templates_folder.clone(), app.services.templates_dir.clone());
    let folder = app.user_templates_dir();
    let templates = match &folder {
        Some(f) if f.is_dir() => {
            wordcraft_engine::cmd::user_templates::list_templates(&f.to_string_lossy()).map(|l| l.templates).unwrap_or_else(|e| {
                log::debug!("personal templates: {e}");
                Vec::new()
            })
        }
        _ => Vec::new(),
    };
    PersonalListing { key, folder, templates }
}

/// New: the whole template gallery with a search box, then the personal templates (#2852); no
/// Recent list (that is on Home and Open). The search covers both.
///
/// The personal listing is read on the first pass of each visit (the body's `appeared`), when the
/// folder setting differs from the one it was read for, and after Refresh; never on every pass,
/// as reading a big or synced folder would stall the window.
pub(super) fn show(ui: &mut Ui, app: &mut WordApp) {
    let ctx = ui.ctx().clone();
    heading(ui, "New");
    let mut query = std::mem::take(&mut app.backstage_state.query);
    let _ = TextField::search(label("Search for templates"), Bind::bound(&mut query)).placeholder(label("Search for templates")).show(ui);
    app.backstage_state.query = query.clone();
    let _ = Stack::new().gap(Gap::Section).fill_width().show(ui, |ui, cx| {
        let key = (app.session.prefs.user_templates_folder.clone(), app.services.templates_dir.clone());
        let stale = app.backstage_state.personal.as_ref().is_none_or(|l| l.key != key);
        if cx.appeared() || stale {
            app.backstage_state.personal = Some(read_listing(app));
        }
        let (folder, personal) = match &app.backstage_state.personal {
            Some(l) => (l.folder.clone(), l.templates.clone()),
            None => (None, Vec::new()),
        };
        let found = matching_templates(&query);
        let personal_found = matching_personal(&query, &personal);
        if found.is_empty() && personal_found.is_empty() {
            let _ = Notice::new(Tone::Neutral, label("No templates match your search.")).show(ui);
            return;
        }
        let store = thumbs::store(&ctx);
        let source = thumbs::TemplateThumbs { region: app.session.region() };
        if !found.is_empty() {
            let (store, source) = (&store, &source);
            let cards = template_cards(&found);
            let items = keyed(&cards, |t: &Template| t.id.clone()).with_text(|t| Cow::Borrowed(t.name.as_str()));
            let out = grid(label("Templates")).show_items(ui, &items, &mut |id, _| {
                let Some(card) = cards.iter().find(|c| &c.id == id) else { return Tile::new(Label::Empty) };
                let thumb = ThumbKey::Template(card.id.clone());
                Tile::new(Label::from(card.name.clone()))
                    .thumb(ThumbAspect::new(PAGE_WIDE, PAGE_HIGH), move |cx, rect| thumbs::paint(cx, rect, store, source, &thumb))
            });
            for id in activated(out.response.events) {
                let _ = app.run("file.new", json!({"template": id}));
                app.ui.backstage = false;
                app.canvas.focus_page(&ctx);
            }
        }
        // Personal: the user templates folder's templates; a folder per category, as in Word.
        let Some(folder) = folder else { return };
        if !query.trim().is_empty() && personal_found.is_empty() {
            return;
        }
        subheading(ui, "Personal");
        if clicked(&Button::new(label("Refresh")).id_salt("personal-refresh").tooltip(folder.to_string_lossy().into_owned()).show(ui)) {
            app.backstage_state.personal = Some(read_listing(app));
        }
        if let Some(error) = app.backstage_state.personal_error.clone() {
            super::error_line(ui, &error);
        }
        if personal.is_empty() {
            let _ = Notice::new(Tone::Neutral, label("Templates you save in your templates folder show up here.")).show(ui);
            let _ = Notice::new(Tone::Neutral, folder.to_string_lossy().into_owned()).show(ui);
            return;
        }
        let mut categories: Vec<&str> = Vec::new();
        for p in &personal_found {
            if !categories.contains(&p.category.as_str()) {
                categories.push(&p.category);
            }
        }
        for category in categories {
            let tiles: Vec<&PersonalTemplate> = personal_found.iter().filter(|p| p.category == category).collect();
            let items = keyed(&tiles, |p: &&PersonalTemplate| p.path.clone()).with_text(|p| Cow::Borrowed(p.label.as_str()));
            let name = if category.is_empty() { label("Personal templates") } else { Label::from(category.to_owned()) };
            let out = grid(name).id_salt(("personal-templates", category.to_owned())).show_items(ui, &items, &mut |path, _| {
                let Some(p) = tiles.iter().find(|p| &p.path == path) else { return Tile::new(Label::Empty) };
                Tile::new(Label::from(p.label.clone())).icon(IconSpec::svg(IconId::new("documentTemplate"))).tooltip(p.path.clone())
            });
            for path in activated(out.response.events) {
                let failed = open_personal(app, &path, &ctx).err();
                app.backstage_state.personal_error = failed;
            }
        }
    });
}

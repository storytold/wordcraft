//! File tab (Backstage): Home/New, Open, Info, Save As, Print, Export, Close, Options, About.

mod home;
mod new_page;
mod pages;
mod thumbs;

use std::cell::{Cell, RefCell};

use craft::behavior::IconId;
use craft::theme::Variant;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::buttons::ControlPlace;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{
    BodyCx, CollectionEvent, Commands as _, Gap, LeastWidth, Pad, Props as _, Scroll, Show as _, ShowContainer as _, Sizing, clicked,
};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::notices::Notice;
use craft::ui::primitives::{HAlign, IconSpec};
use craft::ui::runtime::env::{Label, with_env};
use craft::ui::runtime::inspect;
use craft::ui::style::{SurfaceKind, TypeRole};
use egui::Ui;
use serde_json::json;

use crate::WordApp;
pub(crate) use thumbs::TemplateThumbs;

/// The side menu: page id, label and icon (#1714).
const PAGES: [(&str, &str, &str); 11] = [
    ("home", "Home", "homeFolder"),
    ("new", "New", "blankPage"),
    ("open", "Open", "open"),
    ("info", "Info", "info"),
    ("recover", "Recover", "recover"),
    ("save", "Save", "save"),
    ("saveAs", "Save As", "saveAs"),
    ("print", "Print", "print"),
    ("export", "Export", "export"),
    ("close", "Close", "close"),
    ("options", "Options", "options"),
];

/// The Backstage pages' keytips, as Word's (#4960): Alt, F, then a letter. One badge per page: Word's
/// older Print Preview key (V) is not shown.
pub(crate) const KEYTIPS: [(&str, &str); 10] = [
    ("home", "H"),
    ("new", "N"),
    ("open", "O"),
    ("info", "I"),
    ("save", "S"),
    ("saveAs", "A"),
    ("print", "P"),
    ("export", "E"),
    ("close", "C"),
    ("options", "T"),
];

/// What the File page itself answers to: Escape and the back button close it, Enter prints on the
/// Print sub-page. One page scope (not raw key reads) so a dialog over the page, the file chooser
/// included, takes Escape first, and so the Print page gets Enter with nothing focused: the shell
/// routes keys to the page's own scope only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft::ui::contract::CommandSet)]
#[command(prefix = "backstagePage", scope = "backstagePage")]
enum BackstagePageCommand {
    /// Leave File and return to the document.
    #[command(label = "Back", default = "Escape")]
    Back,
    /// Print the document; acts only while the Print page shows.
    #[command(label = "Print", default = "Enter")]
    Print,
}

/// The scope of the File page, which the shell's page slot is an instance of.
pub(crate) fn scope() -> craft::ui::runtime::commands::CommandScope {
    <BackstagePageCommand as craft::ui::runtime::commands::CommandSet>::SCOPE
}

/// A personal template: what the gallery needs of a listing entry.
type PersonalTemplate = wordcraft_engine::cmd::user_templates::PersonalTemplate;

/// The templates folder setting and the listing File › New shows for it.
#[derive(Clone)]
pub(crate) struct PersonalListing {
    /// The settings the listing was read for; a different folder setting reads it again.
    key: (Option<String>, Option<std::path::PathBuf>),
    /// The folder the listing is of, when there is one.
    folder: Option<std::path::PathBuf>,
    /// Its templates.
    templates: Vec<PersonalTemplate>,
}

/// What the File page keeps for its window between passes: the errors it shows, the personal
/// template listing and the New page's search text.
#[derive(Default)]
pub(crate) struct BackstageState {
    /// Why the last attempt to open a recent document failed.
    pub(crate) open_error: Option<String>,
    /// The Save As page's write-failure message ([`WordApp::fail_save_as`]): this page skips the
    /// status bar, so a failed write (no permission, disk full, a bad path) says why here.
    pub(crate) save_as_error: Option<String>,
    /// Why the last attempt to open a personal template failed.
    pub(crate) personal_error: Option<String>,
    /// The personal templates as last read (when New was shown, the folder setting changed, or
    /// Refresh); `None` reads them on the next pass of New.
    personal: Option<PersonalListing>,
    /// The New page's search text.
    pub(crate) query: String,
}

impl BackstageState {
    /// Forgets every error message, so none comes back when File is opened next time.
    pub(crate) fn clear_errors(&mut self) {
        self.open_error = None;
        self.save_as_error = None;
        self.personal_error = None;
    }
}

/// `text` in the interface language, as a framework label.
fn label(text: &str) -> Label {
    Label::from(tl!(text).to_owned())
}

/// A page's heading: the page's largest text, in the role the framework's start screen gives the Home page's
/// greeting, so every File page's title has one size (production: 26 pt semibold over 15 pt sections).
fn heading(ui: &mut Ui, text: &str) {
    // Production's page title is large (26 pt semibold), well above its 15 pt section headings: the one role above
    // the section headings' is the display role.
    let _ = Text::new(label(text)).role(TypeRole::Display).show(ui);
}

/// A section heading inside a page, a step under the page's title: production's 15 pt semibold, the heading role.
fn subheading(ui: &mut Ui, text: &str) {
    let _ = Text::new(label(text)).role(TypeRole::Heading).show(ui);
}

/// An error message on a page.
fn error_line(ui: &mut Ui, text: &str) {
    let _ = Notice::new(Tone::Bad, text.to_owned()).show(ui);
}

/// The keys a collection's events activated, in order.
fn activated<K, E>(events: Vec<CollectionEvent<K, E>>) -> Vec<K> {
    events
        .into_iter()
        .filter_map(|event| match event {
            CollectionEvent::Activate(key) => Some(key),
            _ => None,
        })
        .collect()
}

/// What the side menu asked for this pass; the page acts after the menu is shown, so no closure of
/// the menu holds the app.
#[derive(Default)]
struct MenuChoice {
    page: RefCell<Option<&'static str>>,
    about: Cell<bool>,
    discord: Cell<bool>,
}

/// The File page, shown in the shell's page slot: the side menu and the active page. Without a
/// document only the Home, New, Open and Options pages work. Back and Escape close it.
pub(crate) fn show(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp) {
    let ctx = ui.ctx().clone();
    // No document open (#1129): only the pages that open or create one, and Options.
    let has_document = app.has_document();
    if !has_document && !works_without_document(&app.ui.backstage_page) {
        app.ui.backstage_page = "home".into();
    }
    while let Some((command, _)) = cx.take_command::<BackstagePageCommand>() {
        match command {
            BackstagePageCommand::Back => {
                let _ = app.run("ui.backstage", json!({"value": false}));
                app.canvas.focus_page_after_pass();
            }
            // PLAN: `print_requested(app, ctx)` is chrome-w10's new `pub(crate)` fn in print_page.rs (S-L263).
            BackstagePageCommand::Print if app.ui.backstage_page == "print" => crate::print_page::print_requested(app, &ctx),
            BackstagePageCommand::Print => {}
        }
    }
    // The page band, the window's move band, lies over the top of the window; the controls start below it. The side
    // menu's back button clears it. The page's own margin (production's 30 pt over its title) already reaches below a
    // title bar's height, so only what the band takes past that margin is cleared there.
    let band = cx.shell().and_then(|shell| shell.page_band()).map_or(0.0, |band| band.rect.height());
    let page_band = (band - craft::ui::contract::pad_of(&craft::ui::runtime::env::Env::of(ui).tokens, Pad::Page)).max(0.0);
    let choice = MenuChoice::default();
    let current = app.ui.backstage_page.clone();
    let show_discord = app.ui.show_discord;
    // The row starts at the reading start, so the side menu is at the right in a mirrored interface.
    let _ = Row::new().fill().show(ui, |ui, _| {
        side_menu(ui, band, &current, has_document, show_discord, &choice);
        let print = current == "print";
        // Stacked rows' spacing between a page's controls, as production's (Browse… over Browse in WordCraft…).
        // The page stands on the window's background (production's `t.ribbon`, #f9f9f9 light), not the white panel,
        // a page's margin off the window and the side menu (production's 40 pt sides, 30 pt top).
        let mut body = Stack::new().fill().pad(Pad::Page).gap(Gap::Stack).surface(SurfaceKind::Background);
        // The Print page fills the pane and scrolls its settings and its preview itself (#1156).
        if !print {
            body = body.scroll(Scroll::vertical());
        }
        let _ = body.show(ui, |ui, _| {
            ui.add_space(page_band);
            page_body(ui, app, &current);
        });
    });
    if let Some(id) = choice.page.take() {
        go_to_page(app, id);
    }
    if choice.about.get() {
        let _ = crate::dialogs::open(app, "about", &json!({}));
    }
    if choice.discord.get() {
        let _ = app.run("ui.discord", json!({}));
    }
}

/// The active page's body.
fn page_body(ui: &mut Ui, app: &mut WordApp, page: &str) {
    // The personal templates are read once per visit to New: a page that is not New drops the
    // listing, so the next visit reads the folder again (a stale listing is re-read by New).
    if page != "new" {
        app.backstage_state.personal = None;
    }
    match page {
        "home" => home::home(ui, app),
        "new" => new_page::show(ui, app),
        "open" => home::open(ui, app),
        "saveAs" => pages::save_as(ui, app),
        "info" => pages::info(ui, app),
        "recover" => pages::recovery(ui, app),
        "print" | "export" => pages::export(ui, app),
        "options" => crate::options::page(app, ui),
        _ => home::home(ui, app),
    }
}

/// The side menu: Back, one button per page (the page buttons are one inspected region, from which
/// the keytips take their rects), then About and the community link unless the user turned the
/// community button off, as the title bar's pill does.
fn side_menu(ui: &mut Ui, band: f32, current: &str, has_document: bool, show_discord: bool, choice: &MenuChoice) {
    // At least the side menu's width and as tall as the window, About at its foot (production's 200 pt full-height
    // rail); its rows are the side menu place's (production's 38 pt rows of 14 pt names).
    let _ = Stack::new()
        .sizing(Sizing::FillHeight)
        .least_width(LeastWidth::SideMenu)
        .surface(SurfaceKind::Brand)
        .label(label("File"))
        .gap(Gap::None)
        .scroll(Scroll::vertical())
        .header(move |ui, _| {
            ui.add_space(band);
            let _ = IconButton::command(IconSpec::svg(IconId::new("chevronLeft")), BackstagePageCommand::Back).show(ui);
        })
        // The community link above About, both at the reading start, as production's bottom links: rows of the side
        // menu place too, so they read in the rail's white ink over the brand colour, not a panel button's dark ink.
        .footer(move |ui, _| {
            with_env(
                ui,
                |overrides| {
                    overrides.values.set(ControlPlace::SideMenu);
                },
                |ui| footer_links(ui, show_discord, choice),
            );
        })
        .show(ui, |ui, _| {
            let _ = inspect::record(ui, egui::Id::unique("wordcraft-backstage-pages"), |ui| {
                // The rows are the side menu place's: production's 38 pt rows of 14 pt names.
                with_env(
                    ui,
                    |overrides| {
                        overrides.values.set(ControlPlace::SideMenu);
                    },
                    |ui| page_rows(ui, current, has_document, choice),
                );
            });
        });
}

/// The side menu's foot: the community link (unless the user turned it off) above About.
fn footer_links(ui: &mut Ui, show_discord: bool, choice: &MenuChoice) {
    if show_discord
        && clicked(
            &Button::new(label("Discord community"))
                .id_salt("backstage-discord")
                .variant(Variant::GHOST)
                .sizing(Sizing::FillWidth)
                .align(HAlign::Start)
                .show(ui),
        )
    {
        choice.discord.set(true);
    }
    if clicked(
        &Button::new(label("About")).id_salt("backstage-about").variant(Variant::GHOST).sizing(Sizing::FillWidth).align(HAlign::Start).show(ui),
    ) {
        choice.about.set(true);
    }
}

/// The side menu's page rows, one full-width button per page.
fn page_rows(ui: &mut Ui, current: &str, has_document: bool, choice: &MenuChoice) {
    for (id, name, icon) in PAGES {
        // No document open (#1129): Info, Save, Save As, Print and Export are greyed out.
        let on = has_document || works_without_document(id);
        let row = Button::new(label(name))
            .id_salt(("backstage-page", id))
            .icon(crate::icons::id(icon))
            .variant(Variant::GHOST)
            .sizing(Sizing::FillWidth)
            // Icon and name at the reading start of the full-width row, as production's menu.
            .align(HAlign::Start)
            .selected(current == id)
            .enabled(on)
            .show(ui);
        if clicked(&row) {
            *choice.page.borrow_mut() = Some(id);
        }
    }
}

/// Opens a side-menu page, as a click on it (or its keytip) does. Without a document only the
/// pages that work without one open.
pub(crate) fn go_to_page(app: &mut WordApp, id: &str) {
    if !app.has_document() && !works_without_document(id) {
        return;
    }
    match id {
        // A document without a file goes to the Save As page (as in Word) instead
        // of `file.save`, which would also open the native Save As dialog.
        // In the browser a save is a download, so it saves right away.
        "save" if app.session.path.is_none() && !app.browser_saves() => {
            app.ui.backstage_page = "saveAs".into();
        }
        "save" => {
            let r = app.run("file.save", json!({}));
            if r.is_ok_and(|v| v.get("saved").and_then(serde_json::Value::as_bool) == Some(true) || v.get("downloaded").is_some()) {
                app.ui.backstage = false;
                app.canvas.focus_page_after_pass();
            } else {
                app.ui.backstage_page = "saveAs".into();
            }
        }
        "saveAs" => {
            app.ui.backstage_page = "saveAs".into();
        }
        // Not a page: `file.close` asks Save / Don't Save / Cancel first when there
        // are unsaved changes, and Cancel leaves the Backstage where it was.
        "close" => {
            let _ = app.run("file.close", json!({}));
        }
        _ => app.ui.backstage_page = id.into(),
    }
}

/// The Backstage pages that work while no document is open (#1129).
fn works_without_document(page: &str) -> bool {
    matches!(page, "home" | "new" | "open" | "options")
}

/// The built-in templates in gallery order: display label and the `file.new` template id.
const TEMPLATES: [(&str, &str); 5] =
    [("Blank document", "blank"), ("Studio handbook (sample)", "sample"), ("Letter", "letter"), ("Résumé", "resume"), ("Report", "report")];

/// How many templates Home shows before "More templates" (the rest are on New).
const HOME_TEMPLATES: usize = 4;

/// The templates whose English or translated label, or id, contains `query` (case-insensitive;
/// the id lets "resume" find "Résumé"); all of them for an empty query.
fn matching_templates(query: &str) -> Vec<(&'static str, &'static str)> {
    let q = query.trim().to_lowercase();
    TEMPLATES
        .into_iter()
        .filter(|(label, id)| {
            q.is_empty() || id.contains(&q) || label.to_lowercase().contains(&q) || crate::i18n::tc("template", label).to_lowercase().contains(&q)
        })
        .collect()
}

/// The personal templates whose name, title or category contains `query` (case-insensitive);
/// all of them for an empty query.
fn matching_personal(query: &str, list: &[PersonalTemplate]) -> Vec<PersonalTemplate> {
    let q = query.trim().to_lowercase();
    let has = |s: &str| s.to_lowercase().contains(&q);
    list.iter().filter(|p| q.is_empty() || has(&p.label) || has(&p.category) || p.title.as_deref().is_some_and(has)).cloned().collect()
}

/// Make a new document from the personal template at `path` and close File; a template that
/// can't be read keeps File open so the gallery can say why.
fn open_personal(app: &mut WordApp, path: &str) -> Result<(), String> {
    app.run("file.newFromTemplate", json!({"path": path}))?;
    app.ui.backstage = false;
    app.canvas.focus_page_after_pass();
    Ok(())
}

/// Open a file from the Recent list. A file that can't be opened (deleted, moved, unreadable)
/// leaves File open so the list can say why, rather than closing it on a greyed-out window.
pub(crate) fn open_recent(app: &mut WordApp, path: &str) -> Result<(), String> {
    app.open_file(path, None)?;
    app.ui.backstage = false;
    Ok(())
}

#[cfg(test)]
mod tests;

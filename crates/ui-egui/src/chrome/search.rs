//! Command search: a `CommandPalette` in its in-place face, over the registry's commands.

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, Props as _, Show as _, TooltipSpec};
use craft::ui::menu::HelpEntry;
use craft::ui::patterns::panels::{CommandPalette, PaletteFace};
use craft::ui::runtime::env::{Label, WordTemplate};
use craft::ui::runtime::inspect;
use egui::Ui;
use serde_json::json;

use crate::WordApp;

/// One help entry for each registry command Search offers: its label, with the English label so a search in
/// English finds it in any language. Paper Color opens the selector in Options instead of running with no
/// value, since Search is an interactive entry point.
pub(crate) fn help_entries(app: &WordApp) -> Vec<HelpEntry> {
    app.session
        .registry
        .all()
        .iter()
        .filter(|c| !crate::dialogs::NOT_IN_SEARCH.contains(&c.id))
        .map(|c| {
            let command =
                if c.id == "view.paperColor" { CommandRef::with_params("file.options", json!({"pane": "general"})) } else { CommandRef::new(c.id) };
            HelpEntry::new(command, tl!(c.label).to_string()).with_english(c.label.to_string())
        })
        .collect()
}

/// Whether this window shows the search field: in the title bar, unless the host left it out ([`crate::Chrome`]),
/// or in Immersive's screen bar. Focus shows neither.
pub(crate) fn has_field(app: &WordApp) -> bool {
    match crate::chrome::Screen::of(&app.session.view) {
        crate::chrome::Screen::Normal => app.chrome.title_bar,
        crate::chrome::Screen::FullScreen => true,
        crate::chrome::Screen::Focus => false,
    }
}

/// Opens the search in this window's field. `false` when it shows none, so the Search dialog takes over.
pub(crate) fn open(app: &mut WordApp) -> bool {
    if !has_field(app) {
        return false;
    }
    app.chrome_open.search = true;
    true
}

/// Whether the search is being typed in: Escape closes it, not Full Screen.
pub(crate) fn is_open(app: &WordApp) -> bool {
    app.chrome_open.search
}

/// Shows the search field, and its results while it is open, as the inspected region the keytips badge; returns the
/// rect it took. `compact` (a narrow window, #5886) shows the magnifier alone, and the search opens as a box over
/// the window, since the bar has no room for the field.
pub(super) fn show(app: &mut WordApp, ui: &mut Ui, compact: bool) -> egui::Rect {
    // Built in every pass the field runs: the palette opens inside its own `show`, so the entries are there before
    // it knows. The list lives for the pass only.
    let entries = help_entries(app);
    let suggested: Vec<CommandRef> = crate::dialogs::SEARCH_SUGGESTIONS.iter().map(|id| CommandRef::new(*id)).collect();
    let mut open = app.chrome_open.search;
    let name = tl!("Search commands and help").to_string();
    let ((rect, out), _) = inspect::record(ui, crate::keytips::search_region(), |ui| {
        let mut rect = None;
        if compact {
            let button = IconButton::new(IconId::new("wordcraft.search")).label(name.clone()).tooltip(TooltipSpec::new(name.clone())).show(ui);
            if button.response.clicked() {
                open = true;
            }
            rect = Some(button.response.rect);
        }
        let out = CommandPalette::new(name.clone(), &entries, Bind::bound(&mut open))
            .face(if compact { PaletteFace::Dialog } else { PaletteFace::InPlace })
            .placeholder(Label::from(name.clone()))
            .suggested((Label::from(tl!("Suggested").to_string()), suggested))
            .fallback(WordTemplate::new(tl!("Find “{text}” in the document")))
            .show(ui);
        (rect.unwrap_or(out.response.rect), out)
    });
    app.chrome_open.search = open;
    if let Some(text) = out.fallback {
        crate::dialogs::find_in_navigation_pane(app, ui.ctx(), &text);
    }
    rect
}

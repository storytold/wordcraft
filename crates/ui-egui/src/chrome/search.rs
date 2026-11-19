//! Command search: a `CommandPalette` in its in-place face, over the registry's commands.

use craft::keymap::CommandRef;
use craft::ui::contract::Bind;
use craft::ui::menu::search::HelpEntry;
use craft::ui::patterns::{CommandPalette, PaletteFace};
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
            let command = if c.id == "view.paperColor" {
                CommandRef::with_params("file.options", json!({"pane": "general"}))
            } else {
                CommandRef::new(c.id)
            };
            HelpEntry::new(command, tl!(c.label).to_string()).with_english(c.label.to_string())
        })
        .collect()
}

/// Opens the search where this window shows its field (the title bar, or Immersive's screen bar). `false`
/// when it shows none: Focus hides it, so the Search dialog takes over.
pub(crate) fn open(app: &mut WordApp) -> bool {
    if matches!(crate::chrome::Screen::of(&app.session.view), crate::chrome::Screen::Focus) {
        return false;
    }
    app.chrome.search = true;
    true
}

/// Whether the search is being typed in: Escape closes it, not Full Screen.
pub(crate) fn is_open(app: &WordApp) -> bool {
    app.chrome.search
}

/// Shows the search field, and its results while it is open, as the inspected region the keytips badge.
pub(super) fn show(app: &mut WordApp, ui: &mut Ui) {
    // Built in every pass the field runs: the palette opens inside its own `show`, so the entries are there before
    // it knows. The list lives for the pass only.
    let entries = help_entries(app);
    let suggested: Vec<CommandRef> = crate::dialogs::SEARCH_SUGGESTIONS.iter().map(|id| CommandRef::new(*id)).collect();
    let mut open = app.chrome.search;
    let (out, _) = inspect::record(ui, crate::keytips::search_region(), |ui| {
        CommandPalette::new(tl!("Search commands and help").to_string(), &entries, Bind::bound(&mut open))
            .face(PaletteFace::InPlace)
            .placeholder(Label::from(tl!("Search commands and help").to_string()))
            .suggested((Label::from(tl!("Suggested").to_string()), suggested))
            .fallback(WordTemplate::new(tl!("Find “{text}” in the document")))
            .show(ui)
    });
    app.chrome.search = open;
    if let Some(text) = out.fallback {
        crate::dialogs::find_in_navigation_pane(app, ui.ctx(), &text);
    }
}

//! Search Commands: the palette over every command, as a dialog. A picked command fires through
//! the board as `FireSource::Palette`; only the "Find … in the document" row comes back here.

use craft::keymap::CommandRef;
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::menu::HelpEntry;
use craft::ui::patterns::panels::CommandPalette;
use craft::ui::runtime::env::{Label, WordTemplate};
use serde_json::Value;

use super::find::{SEARCH_SUGGESTIONS, find_in_navigation_pane};
use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

/// The Search Commands dialog; its answer is the query of the fallback row, when that was picked.
pub(super) struct SearchCommandsDialog {
    entries: Vec<HelpEntry>,
    open: bool,
    suggested: Vec<CommandRef>,
    fallback: WordTemplate,
}

impl SearchCommandsDialog {
    const KEY: &'static str = "commands";

    fn new(entries: Vec<HelpEntry>) -> Self {
        Self {
            entries,
            open: true,
            suggested: SEARCH_SUGGESTIONS.iter().map(|id| CommandRef::new(*id)).collect(),
            fallback: WordTemplate::new(tl!("Find “{text}” in the document")),
        }
    }
}

impl StackDialog for SearchCommandsDialog {
    type Result = Option<String>;

    fn id(&self) -> DialogId {
        dialog_id(Self::KEY)
    }

    fn key(&self) -> Option<&str> {
        Some(Self::KEY)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<Option<String>>> {
        let out = CommandPalette::new(tl!("Search Commands"), &self.entries, Bind::bound(&mut self.open))
            .suggested((Label::from(tl!("Suggested")), self.suggested.clone()))
            .fallback(self.fallback.clone())
            .show(ui);
        if let Some(text) = out.fallback {
            return Some(DialogResult::Ok(Some(text)));
        }
        out.closed.map(|_| DialogResult::Ok(None))
    }
}

/// Opens Search Commands, or `None` when the title bar's in-place field took the request. This is
/// the only place that decides between the two faces.
pub(super) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    // PLAN: assumed `chrome::search::open(app) -> bool` and `help_entries(app) -> Vec<HelpEntry>` (chrome-w8).
    if crate::chrome::search::open(app) {
        return None;
    }
    let entries = crate::chrome::search::help_entries(app);
    Some(open_with(app, parent, SearchCommandsDialog::new(entries), DialogHooks { prepare: None, settle }))
}

fn settle(app: &mut WordApp, ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(result) = app.dialog_host.take_by_id::<SearchCommandsDialog>(id) else {
        return Settle::Keep;
    };
    if let DialogResult::Ok(Some(text)) = result {
        find_in_navigation_pane(app, ctx, text.trim());
    }
    Settle::Done
}

//! Insert › Library (#2868): the Library pane the Craft apps share ([`craft_library_egui`]), with
//! WordCraft's words and colours, over the engine's `library.*` commands. Its Pictures tab shows
//! the pictures in the user's Library folders; clicking one inserts it at the selection, as
//! Insert › Pictures does, while the document stays open for editing.
//!
//! Add Folder… asks for a folder in the system's dialog (the built-in browser where there is none,
//! #576) without waiting for it: the answer adds the folder on a later frame
//! ([`AfterPick::LibraryFolder`](crate::file_dialogs::AfterPick::LibraryFolder)). Where nothing
//! can pick a folder (the web) it shows the pane's path field. A tab of WordCraft's own (icons,
//! diagrams, charts) would join the strip in [`LABELS`] and be drawn in `WordHost`'s `Host::tab`.

use std::borrow::Cow;

use craft_library_egui::{Host, Labels, Palette};
pub use craft_library_egui::{ITEMS, MAX_SHOWN_FILES, State};
use egui::Ui;
use serde_json::Value;

use crate::WordApp;
use crate::file_dialogs::{AfterPick, FileDialogRequest};
use crate::theme::Tokens;

/// WordCraft's words in the shared pane, in English: every one goes through the interface catalog
/// in `WordHost`'s `Host::text`. The nouns are the commands' keys and are never shown. A picture file
/// holds one picture, so the pictures share one grid, each named under its thumbnail.
pub const LABELS: Labels = Labels {
    nouns: wordcraft_engine::library::NOUNS,
    tabs: &[(ITEMS, "Pictures")],
    item: "Picture",
    insert_item: "Insert picture",
    insert_shapes: None,
    insert_element: None,
    search_hint: "Search pictures",
    add_folder_tip: "Show the pictures in a folder: PNG, JPEG, GIF, WebP, BMP and TIFF files",
    empty: "Add a folder of pictures to see them here. Click a picture to insert it.",
    not_in_browser: "The Library shows the pictures in folders on this computer, which the browser version can't read. Use the desktop app to browse a Library.",
    aspect: 0.75,
    one_per_file: true,
};

/// Every word the pane can show, WordCraft's and the shared pane's own, in English: what the
/// missing-translation report lists for translators even when no test reaches the state that
/// shows it.
pub fn words() -> impl Iterator<Item = &'static str> {
    let l = LABELS;
    let own = [l.item, l.insert_item, l.search_hint, l.add_folder_tip, l.empty, l.not_in_browser];
    l.tabs.iter().map(|(_, label)| *label).chain(own).chain(craft_library_egui::words::ALL.iter().copied())
}

/// The pane's colours from the theme.
pub fn palette(t: &Tokens) -> Palette {
    Palette { text_dim: t.text_dim, text_faint: t.text_disabled, separator: t.border, control: t.input, border: t.border, accent: t.accent }
}

/// The app, as the shared pane sees it.
pub struct WordHost<'a>(pub &'a mut WordApp);

impl Host for WordHost<'_> {
    /// An insert, or adding or removing a folder, as the person's own command: a failure shows on
    /// the status bar, and a new folder list is saved at once, so it survives a crash.
    fn act(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let r = self.0.run(id, params);
        if r.is_ok() {
            match id {
                "library.addFolder" | "library.removeFolder" => {
                    let _ = self.0.save_prefs_now();
                }
                // Typing carries on in the document after a picture goes in.
                "library.insert" => self.0.canvas.want_focus = true,
                _ => {}
            }
        }
        r
    }
    fn ask(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.0.session.run(id, &params).map_err(|e| e.to_string())
    }
    fn folders(&self) -> Vec<String> {
        self.0.session.prefs.library_folders.clone()
    }
    /// The pane asks synchronously, but WordCraft's dialogs answer on a later frame: the dialog is
    /// started and the pane told "cancelled" (it does nothing more), and the answer adds the folder
    /// and shows it. No picker (the web): `None`, so the pane takes a typed path.
    fn pick_folder(&mut self) -> Option<Option<String>> {
        if !self.0.has_folder_picker() {
            return None;
        }
        let req = FileDialogRequest::Folder { purpose: "library".into() };
        let _ = self.0.ask_file(req, AfterPick::LibraryFolder);
        Some(None)
    }
    fn can_place(&self) -> bool {
        self.0.has_document()
    }
    fn report(&mut self, message: String) {
        self.0.status(message);
    }
    /// The interface catalog's wording; a `{name}` placeholder stays for the pane to fill in.
    fn text<'a>(&self, english: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(tl!(english))
    }
}

/// Draw the pane's body.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let pal = palette(&Tokens::get(ui.ctx()));
    // The pane's state is the app's; it is lent to the pane while the app is the pane's host. The
    // pane scrolls its own grid, so its folder and search controls stay in view.
    let mut state = std::mem::take(&mut app.library);
    craft_library_egui::show(ui, &mut state, &mut WordHost(app), &LABELS, &pal);
    // A folder picked while the state was lent (a host that answered Add Folder… at once) was
    // shown on the stand-in; it is shown on the pane's own state.
    let picked = std::mem::take(&mut app.library).folder;
    app.library = state;
    if picked.is_some() {
        app.library.folder = picked;
    }
}

#[cfg(test)]
mod ui_tests;

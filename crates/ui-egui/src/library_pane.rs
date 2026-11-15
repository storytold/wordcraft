//! Insert › Library (#2868): the Library pane the Craft apps share ([`craft_library_egui`]), with
//! WordCraft's words and colours, over the engine's `library.*` commands. Its Pictures tab shows
//! the pictures in the user's Library folders; clicking one inserts it at the selection, as
//! Insert › Pictures does, while the document stays open for editing.
//!
//! WordCraft has no folder picker (its folder options take a typed path too), so Add Folder…
//! shows the pane's path field. A tab of WordCraft's own (icons, diagrams, charts) would join the
//! strip in `tabs` and be drawn in `WordHost`'s `Host::tab`.

use craft_library_egui::{Host, Labels, Palette};
pub use craft_library_egui::{ITEMS, MAX_SHOWN_FILES, State};
use egui::Ui;
use serde_json::Value;

use crate::WordApp;
use crate::theme::Tokens;

/// Words the shared pane draws itself, with no way yet for the app to give them in the interface
/// language: its folder drop-down and buttons. They stay English until craft-library-egui takes
/// them from the app too (to raise for storytold/craft-libs#1584); the interface audit
/// (`i18n::audit`) lets exactly these through, and only while the Library pane is open.
pub const SHARED_ENGLISH: [&str; 4] = ["All folders", "Add Folder…", "Remove", "Refresh"];

/// The tab strip in the interface language: `Labels::tabs` must outlive the frame, so each
/// translation of it is made once and kept (one per language the window is shown in, at most
/// [`MAX_TAB_STRIPS`]).
fn tabs() -> &'static [(&'static str, &'static str)] {
    type Strip = &'static [(&'static str, &'static str)];
    static STRIPS: std::sync::Mutex<Vec<Strip>> = std::sync::Mutex::new(Vec::new());
    const ENGLISH: Strip = &[(ITEMS, "Pictures")];
    let label: &'static str = tl!("Pictures");
    let Ok(mut strips) = STRIPS.lock() else { return ENGLISH };
    if let Some(strip) = strips.iter().find(|s| s.first().is_some_and(|(_, l)| *l == label)) {
        return strip;
    }
    if strips.len() >= MAX_TAB_STRIPS {
        return ENGLISH;
    }
    let strip: Strip = Box::leak(Box::new([(ITEMS, label)]));
    strips.push(strip);
    strip
}

/// More translations of the tab strip than there are interface languages: past it, English.
const MAX_TAB_STRIPS: usize = 256;

/// WordCraft's words in the shared pane, in the interface language. The nouns are the commands'
/// keys, so they stay as they are.
pub fn labels() -> Labels {
    Labels {
        nouns: wordcraft_engine::library::NOUNS,
        tabs: tabs(),
        item: tl!("Picture"),
        insert_item: tl!("Insert picture"),
        insert_shapes: None,
        insert_element: None,
        search_hint: tl!("Search pictures"),
        add_folder_tip: tl!("Show the pictures in a folder: PNG, JPEG, GIF, WebP, BMP and TIFF files"),
        empty: tl!("Add a folder of pictures to see them here. Click a picture to insert it."),
        not_in_browser: tl!(
            "The Library shows the pictures in folders on this computer, which the browser version can't read. Use the desktop app to browse a Library."
        ),
        aspect: 0.75,
    }
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
    fn pick_folder(&mut self) -> Option<Option<String>> {
        None
    }
    fn can_place(&self) -> bool {
        self.0.has_document()
    }
    fn report(&mut self, message: String) {
        self.0.status(message);
    }
}

/// Draw the pane's body.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let pal = palette(&Tokens::get(ui.ctx()));
    let labels = labels();
    // The pane's state is the app's; it is lent to the pane while the app is the pane's host.
    let mut state = std::mem::take(&mut app.library);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        craft_library_egui::show(ui, &mut state, &mut WordHost(app), &labels, &pal);
    });
    app.library = state;
}

#[cfg(test)]
mod ui_tests;

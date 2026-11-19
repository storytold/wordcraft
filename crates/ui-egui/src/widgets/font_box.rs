//! The font box: Word's plain font picker over the app's catalog and its one recent-fonts list.

use std::sync::Arc;

use craft::font_manager::Catalog;
use craft::font_manager::picker::{FontPicker, FontPickerKind, FontPickerOutput, FontPickerSettings, MissingAdvice};
use craft::ui::contract::{Bind, Show as _};
use craft::ui::runtime::env::Label;
use egui::Ui;
use serde_json::json;

use crate::WordApp;

/// A catalog a dialog form holds; forms compare equal when they hold the same catalog.
#[derive(Clone, Default)]
pub(crate) struct CatalogRef(pub Option<Arc<Catalog>>);

impl std::fmt::Debug for CatalogRef {
    /// The catalog itself is large and has no `Debug`: forms only need to say whether they hold one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CatalogRef").field(&self.0.is_some()).finish()
    }
}

impl PartialEq for CatalogRef {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl CatalogRef {
    /// The catalog, when one is held.
    pub(crate) fn get(&self) -> Option<&Arc<Catalog>> {
        self.0.as_ref()
    }
}

/// The action rows after the font families, in the order `FontPickerOutput::action` indexes them:
/// Add Font…, and in the browser Load System Installed Fonts….
fn actions() -> Vec<Label> {
    let add = Label::from(tl!("Add Font…").to_string());
    let load = cfg!(target_arch = "wasm32").then(|| Label::from(tl!("Load System Installed Fonts…").to_string()));
    std::iter::once(add).chain(load).collect()
}

/// The command an action row runs.
fn action_command(index: usize) -> Option<&'static str> {
    match index {
        0 => Some("font.add"),
        #[cfg(target_arch = "wasm32")]
        1 => Some("font.loadSystem"),
        _ => None,
    }
}

/// One font box configuration: Word's plain list of recent fonts, the `actions` rows and all fonts,
/// with the note for a shown font that is not installed (`shown_as` names the face drawn instead).
pub fn font_picker<'a>(
    label: impl Into<Label>,
    family: Bind<'a, String>,
    catalog: &'a Arc<Catalog>,
    settings: Bind<'a, FontPickerSettings>,
    actions: &'a [Label],
    shown_as: Option<&'a str>,
) -> FontPicker<'a> {
    let picker = FontPicker::new(label, family, catalog).kind(FontPickerKind::Plain).settings(settings).actions(actions);
    let picker = if actions.is_empty() { picker } else { picker.advice(MissingAdvice::AddFont) };
    match shown_as {
        Some(face) => picker.shown_as(face),
        None => picker,
    }
}

/// The ribbon's and mini toolbar's font box over `family`. Add Font… and Load System Installed
/// Fonts… run their commands here; the output carries the pick and its hover preview.
pub fn font_box(ui: &mut Ui, app: &mut WordApp, label: impl Into<Label>, family: &mut String) -> FontPickerOutput {
    let warning = app.font_box_warning(family);
    let rows = actions();
    let catalog = Catalog::global();
    // PLAN: assumed `UiState::font_picker: FontPickerSettings`, replacing `recent_fonts` (app-hub).
    let out = font_picker(label, Bind::bound(family), catalog, Bind::bound(&mut app.ui.font_picker), &rows, warning.as_deref()).show(ui);
    if let Some(command) = out.action.and_then(action_command) {
        let _ = app.run(command, json!({}));
    }
    out
}

impl WordApp {
    /// The font catalog every font box lists: the process's one catalog.
    pub(crate) fn font_catalog(&self) -> Arc<Catalog> {
        Arc::clone(Catalog::global())
    }

    /// The font pickers' remembered state: stars and the one recent-fonts list.
    pub(crate) fn font_picker_settings(&self) -> FontPickerSettings {
        self.ui.font_picker.clone()
    }

    /// Stores the font pickers' remembered state a dialog's picker changed.
    pub(crate) fn set_font_picker_settings(&mut self, settings: FontPickerSettings) {
        self.ui.font_picker = settings;
    }
}

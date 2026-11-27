//! Dialogs. Every dialog is a framework `StackDialog` on the window's `dialog_host`, opened by its name through
//! [`open`]; [`host`] drives them. Every dialog ends by running commands (or shows one's result), so agents get the
//! same result without the dialog.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::AboutTab;
use serde_json::Value;

use crate::WordApp;

mod bookmarks;
pub(crate) mod chrome;
mod customize_keyboard;
mod enclose;
mod find;
mod find_replace;
mod font;
mod forms;
pub(crate) mod host;
mod link;
mod macros;
mod mailings;
pub(crate) mod merge_dialogs;
mod page_setup;
mod paragraph;
mod paste_special;
mod phonetic;
pub(crate) mod prompts;
mod search_commands;
mod simple_forms;
pub(crate) mod style_forms;
mod styles;

pub(crate) use find::{NOT_IN_SEARCH, SEARCH_SUGGESTIONS, find_in_navigation_pane};
pub use forms::{CustomPropertiesForm, TableForm, TableProperties};
#[cfg(test)]
pub(crate) use host::close_all;
#[cfg(test)]
pub(crate) use mailings::{FilterRow, RecipientDraft, RecipientFilter, RecipientListDialog, hold_rows, merge_order, visible_recipients};
pub(crate) use page_setup::PAPER_LABELS;

/// Opens the dialog `name` for a script, a button or a command, with `params`; the module of the name reads what it
/// needs of them. Returns the dialog's id, or `None` when no dialog opened (an unknown name, nothing to show, or a
/// pane such as Restrict Editing).
pub fn open(app: &mut WordApp, name: &str, params: &Value) -> Option<DialogId> {
    route(app, name, params, None)
}

/// The name table: the one place a dialog name picks its module. `parent` is the dialog the new one opens over.
fn route(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "about" => host::open_about(app, AboutTab::About),
        "contributors" => host::open_about(app, AboutTab::Contributors),
        "models" => host::open_about(app, AboutTab::Models),
        "languageData" => host::open_about(app, AboutTab::App(0)),
        // The Restrict Editing pane is the one name that is not a dialog.
        "restrictEditing" => {
            app.restrict.open = true;
            None
        }
        "fontDownload" => crate::font_download::open(app, name, params, parent),
        "font" => font::open(app, name, params, parent),
        // The font colour's More Colors… (the colour picker keeps its own answer route).
        "colors" => crate::colors::open(app, name, params),
        "phonetic" => phonetic::open(app, name, params, parent),
        "enclose" => enclose::open(app, name, params, parent),
        "defineBullet" => dialogs_bullet::open(app, name, params, parent),
        "paragraph" | "asianTypography" => paragraph::open(app, name, params, parent),
        "tabs" | "borders" | "pageBorders" | "shading" => dialogs_para::open(app, name, params, parent),
        "defineList" | "trackChangesOptions" => dialogs_lists::open(app, name, params, parent),
        "dateTime" | "columns" | "symbol" | "specialCharacters" | "field" | "insertSmartArt" | "changeChartType" | "insertChart" | "chartData"
        | "dropCap" => dialogs_insert::open(app, name, params, parent),
        "pageSetup" => page_setup::open(app, name, params, parent),
        "link" => link::open(app, name, params, parent),
        "bookmark" | "icons" => bookmarks::open(app, name, params, parent),
        "pasteSpecial" => paste_special::open(app, name, params, parent),
        "renameDocument" | "insertTable" | "watermark" | "fieldCode" | "editFieldCode" | "tableCellMargins" => {
            simple_forms::open(app, name, params, parent)
        }
        "wordCount" | "zoom" => dialogs_view::open(app, name, params, parent),
        "tableProperties" | "customProperties" => forms::open(app, name, params, parent),
        "lineNumbers" | "hyphenation" | "manualHyphenation" | "language" | "envelopes" | "labels" | "tableFormula" | "tableSort" | "splitCells"
        | "textToTable" | "tableToText" | "pictureSize" | "pageNumbers" => dialogs_layout::open(app, name, params, parent),
        "caption" | "captionNumbering" | "newLabel" | "crossReference" | "index" | "markEntry" | "toc" | "tocOptions" | "tocStyles"
        | "tableOfFigures" | "sourceManager" | "editSource" | "createSource" => dialogs_refs::open(app, name, params, parent),
        "compare" | "combine" | "autoCorrect" => dialogs_review::open(app, name, params, parent),
        "noteOptions" => dialogs_notes::open(app, name, params, parent),
        "customizeKeyboard" => customize_keyboard::open(app, name, params, parent),
        "customizeRibbon" | "customizeQuickAccess" => dialogs_ribbon::open(app, name, params, parent),
        "macros" => macros::open(app, name, params, parent),
        "controlProperties" => dialogs_controls::open(app, name, params, parent),
        // Search opens in the title bar's box when the window shows it; its module decides.
        "commands" => search_commands::open(app, name, params, parent),
        "find" | "replace" | "goto" => find_replace::open(app, name, params, parent),
        "password" | "encryptPassword" | "discardRecovery" => prompts::open(app, name, params, parent),
        "newStyle" | "modifyStyle" => style_forms::open(app, name, params, parent),
        "manageStyles" => styles::open(app, name, params, parent),
        "templates" | "organizer" => dialogs_templates::open(app, name, params, parent),
        "newTableStyle" | "modifyTableStyle" => dialogs_table_style::open(app, name, params, parent),
        "recipientList" | "newRecipientList" => mailings::open(app, name, params, parent),
        "insertMergeField" | "selectTable" | "findRecipient" | "mergeRecords" | "printRecords" | "ruleIf" | "ruleSkipIf" | "matchFields"
        | "checkErrors" => merge_dialogs::open(app, name, params, parent),
        _ => None,
    }
}

pub mod dialogs_bullet;
pub mod dialogs_chart;
pub mod dialogs_controls;
pub mod dialogs_insert;
pub mod dialogs_layout;
pub mod dialogs_lists;
pub mod dialogs_notes;
pub mod dialogs_para;
pub mod dialogs_refs;
pub mod dialogs_review;
pub mod dialogs_ribbon;
pub mod dialogs_smart_art;
pub mod dialogs_table_style;
pub mod dialogs_templates;
mod dialogs_view;
#[cfg(test)]
mod tests;

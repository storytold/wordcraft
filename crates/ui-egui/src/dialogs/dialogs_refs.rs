//! References dialogs (#402): Caption (with New Label and Numbering), Cross-reference (#916),
//! Index and Mark Index Entry, Custom Table of Contents (with Options and the TOC styles), Table of
//! Figures (#3013), and the Source Manager with Create / Edit Source. Each fills a form from the
//! session and ends in a command (`references.caption`, `insert.crossReference`, `references.index`,
//! `references.markEntry`, `references.toc`, `references.tableOfFigures`, `references.sources`,
//! `references.citation`), so agents get the same result without the dialog. Each form is a
//! [`WordForm`](crate::dialogs::host::WordForm) opened by name through [`open`].

use craft::ui::dialog::DialogId;
use serde_json::Value;

use crate::WordApp;
use crate::dialogs::host::open_form;

mod caption;
mod cross_ref;
mod index;
mod labels;
mod sample;
mod sources;
mod toc;

pub use caption::CaptionForm;
pub use cross_ref::CrossRefForm;
#[cfg(test)]
use cross_ref::ref_types;
pub use index::{IndexForm, MarkForm};
pub use sources::SourceForm;
#[cfg(test)]
use sources::field_mut;
pub use toc::{FiguresForm, TocForm};

/// Opens the References dialog `name`, over `parent` when it is a child. `None` for a name this
/// module does not own.
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "caption" => Some(open_form(app, parent, CaptionForm::read(app))),
        "newLabel" | "captionNumbering" => {
            // The child edits its Caption dialog's form, so a child opened by name opens the Caption first.
            let caption = parent.unwrap_or_else(|| open_form(app, None, CaptionForm::read(app)));
            Some(caption::open_child(app, name, params, caption))
        }
        "crossReference" => Some(cross_ref::open(app, parent, params.get("to").and_then(Value::as_str).unwrap_or("heading"))),
        "index" | "markEntry" => index::open(app, name, params, parent),
        "toc" | "tocOptions" | "tocStyles" | "tableOfFigures" => toc::open(app, name, params, parent),
        "sourceManager" | "createSource" | "editSource" => sources::open(app, name, params, parent),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

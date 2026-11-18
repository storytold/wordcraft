//! References dialogs (#402): Caption (with New Label and Numbering), Cross-reference (#916),
//! Index and Mark Index Entry, Custom Table of Contents (with Options and the TOC styles), Table of
//! Figures (#3013), and the Source Manager with Create / Edit Source. Each fills a form from the
//! session and ends in a command (`references.caption`, `insert.crossReference`, `references.index`,
//! `references.markEntry`, `references.toc`, `references.tableOfFigures`, `references.sources`,
//! `references.citation`), so
//! agents get the same result without the dialog. Shown through [`crate::dialogs::Dialog::Refs`].

use egui::{Sense, Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::{Block, Source, StyleKind};
use wordcraft_engine::cmd::references::FieldCode;

use crate::WordApp;
use crate::dialogs::{Dialog, buttons};
#[cfg(not(target_arch = "wasm32"))]
use crate::file_dialogs::{AfterPick, Asked, FileDialogRequest};
use crate::theme::{Tokens, semibold};

mod caption;
mod common;
mod cross_ref;
mod index;
mod shell;
mod sources;
mod toc;

pub use caption::CaptionForm;
pub use cross_ref::CrossRefForm;
#[cfg(test)]
use cross_ref::ref_types;
pub use index::{IndexForm, MarkForm};
pub use shell::{body, open};
pub use sources::SourceForm;
#[cfg(test)]
use sources::field_mut;
pub use toc::{FiguresForm, TocForm};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "refs", rename_all = "camelCase")]
pub enum RefsDialog {
    Caption(CaptionForm),
    /// Caption › Numbering…; `was` comes back on Cancel.
    CaptionNumbering {
        form: CaptionForm,
        #[serde(skip)]
        was: Box<CaptionForm>,
    },
    /// Caption › New Label….
    NewLabel(CaptionForm),
    CrossRef(CrossRefForm),
    Index(IndexForm),
    MarkEntry(MarkForm),
    Toc(TocForm),
    /// Table of Contents › Options…; `was` comes back on Cancel.
    TocOptions {
        form: TocForm,
        #[serde(skip)]
        was: Box<TocForm>,
    },
    /// Table of Contents › Modify…: the TOC 1–9 styles.
    TocStyles {
        form: TocForm,
        level: u8,
    },
    Figures(FiguresForm),
    Sources {
        selected: Option<usize>,
    },
    Source(Box<SourceForm>),
}

impl RefsDialog {
    pub fn name(&self) -> &'static str {
        match self {
            RefsDialog::Caption(_) => "caption",
            RefsDialog::CaptionNumbering { .. } => "captionNumbering",
            RefsDialog::NewLabel(_) => "newLabel",
            RefsDialog::CrossRef(_) => "crossReference",
            RefsDialog::Index(_) => "index",
            RefsDialog::MarkEntry(_) => "markEntry",
            RefsDialog::Toc(_) => "toc",
            RefsDialog::TocOptions { .. } => "tocOptions",
            RefsDialog::TocStyles { .. } => "tocStyles",
            RefsDialog::Figures(_) => "tableOfFigures",
            RefsDialog::Sources { .. } => "sourceManager",
            RefsDialog::Source(f) if f.edit => "editSource",
            RefsDialog::Source(_) => "createSource",
        }
    }

    /// The window title (English; the caller translates it).
    pub fn title(&self) -> &'static str {
        match self {
            RefsDialog::Caption(_) => "Caption",
            RefsDialog::CaptionNumbering { .. } => "Caption Numbering",
            RefsDialog::NewLabel(_) => "New Label",
            RefsDialog::CrossRef(_) => "Cross-reference",
            RefsDialog::Index(_) => "Index",
            RefsDialog::MarkEntry(_) => "Mark Index Entry",
            RefsDialog::Toc(_) => "Table of Contents",
            RefsDialog::TocOptions { .. } => "Table of Contents Options",
            RefsDialog::TocStyles { .. } => "Table of Contents Styles",
            RefsDialog::Figures(_) => "Table of Figures",
            RefsDialog::Sources { .. } => "Source Manager",
            RefsDialog::Source(f) if f.edit => "Edit Source",
            RefsDialog::Source(_) => "Create Source",
        }
    }
}

#[cfg(test)]
mod tests;

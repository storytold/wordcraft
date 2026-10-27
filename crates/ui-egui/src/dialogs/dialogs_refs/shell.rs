use super::caption::{CaptionForm, caption_ui, new_label_ui, numbering_ui};
use super::cross_ref::{CrossRefForm, cross_ref_ui};
use super::index::{IndexForm, MarkForm, index_ui, mark_ui};
use super::sources::{SourceForm, source_ui, sources_ui};
use super::toc::{FiguresForm, TocForm, figures_ui, toc_options_ui, toc_styles_ui, toc_ui};
use super::*;

/// The dialog `ui.dialog` opens by `name`: `caption`, `crossReference`, `index`, `markEntry`, `toc`,
/// `tableOfFigures`, `sourceManager` or `createSource` (which inserts a citation of the new source).
pub fn open(name: &str, app: &WordApp) -> Option<RefsDialog> {
    Some(match name {
        "caption" => RefsDialog::Caption(CaptionForm::read(app)),
        "crossReference" => RefsDialog::CrossRef(CrossRefForm::read(app, "heading")),
        "index" => RefsDialog::Index(IndexForm::read(app)),
        "markEntry" => RefsDialog::MarkEntry(MarkForm::read(app)),
        "toc" => RefsDialog::Toc(TocForm::read(app)),
        "tableOfFigures" => RefsDialog::Figures(FiguresForm::read(app)),
        "sourceManager" => RefsDialog::Sources { selected: None },
        "createSource" => RefsDialog::Source(Box::new(SourceForm::new(true, false))),
        _ => return None,
    })
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut RefsDialog) -> bool {
    let next = match d {
        RefsDialog::Caption(f) => caption_ui(app, ui, f),
        RefsDialog::CaptionNumbering { form, was } => numbering_ui(ui, form, was),
        RefsDialog::NewLabel(f) => new_label_ui(app, ui, f),
        RefsDialog::CrossRef(f) => cross_ref_ui(app, ui, f),
        RefsDialog::Index(f) => index_ui(app, ui, f),
        RefsDialog::MarkEntry(f) => mark_ui(app, ui, f),
        RefsDialog::Toc(f) => toc_ui(app, ui, f),
        RefsDialog::TocOptions { form, was } => toc_options_ui(app, ui, form, was),
        RefsDialog::TocStyles { form, level } => toc_styles_ui(app, ui, form, level),
        RefsDialog::Figures(f) => figures_ui(app, ui, f),
        RefsDialog::Sources { selected } => sources_ui(app, ui, selected),
        RefsDialog::Source(f) => source_ui(app, ui, f),
    };
    match next {
        Next::Stay => false,
        Next::Close => true,
        Next::Go(n) => {
            *d = *n;
            false
        }
    }
}

pub(super) fn go(d: RefsDialog) -> Next {
    Next::Go(Box::new(d))
}

/// What a dialog does after a frame.
pub(super) enum Next {
    Stay,
    Close,
    Go(Box<RefsDialog>),
}

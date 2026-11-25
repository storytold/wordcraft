//! File › Export › Page images (#7743): the resolution and the pages, before asking where to save.
//! OK runs `file.exportPng` with them and no path; the engine asks the app for the file, and the
//! export after that dialog keeps the choices. Scripts and agents pass `dpi`, `range` and `pages`
//! to `file.exportPng` themselves.

use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, FieldLayout, Show as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::runtime::env::Label;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wordcraft_engine::page_images::DPI_CHOICES;

use super::dialogs_insert::{select_index, text_field};
use super::host::{Asks, WordForm, open_form};
use super::{Value, WordApp};

/// The resolution picked when the dialog opens, DPI: print quality, as the request asks for.
const START_DPI: u32 = 300;

/// Which pages: every page, the one the caret is on, or the ones typed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PagesChoice {
    #[default]
    All,
    Current,
    Custom,
}

impl PagesChoice {
    const ALL: [PagesChoice; 3] = [PagesChoice::All, PagesChoice::Current, PagesChoice::Custom];

    /// `file.exportPng`'s `range` for it.
    fn range(self) -> &'static str {
        match self {
            PagesChoice::All => "all",
            PagesChoice::Current => "current",
            PagesChoice::Custom => "custom",
        }
    }

    fn label(self) -> &'static str {
        match self {
            PagesChoice::All => "All pages",
            PagesChoice::Current => "Current page",
            PagesChoice::Custom => "Pages:",
        }
    }
}

/// The Export as Images form.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExportImagesForm {
    /// The index into [`DPI_CHOICES`] picked.
    dpi: usize,
    /// The index into [`PagesChoice::ALL`] picked.
    pages: usize,
    /// The pages typed for [`PagesChoice::Custom`] (`1-3, 5`).
    pages_text: String,
}

impl Default for ExportImagesForm {
    fn default() -> Self {
        // The browser saves one download at a time, so there the current page is the start.
        let pages = if cfg!(target_arch = "wasm32") { PagesChoice::Current } else { PagesChoice::All };
        ExportImagesForm {
            dpi: DPI_CHOICES.iter().position(|&d| d == START_DPI).unwrap_or_default(),
            pages: PagesChoice::ALL.iter().position(|&c| c == pages).unwrap_or_default(),
            pages_text: String::new(),
        }
    }
}

impl ExportImagesForm {
    fn choice(&self) -> PagesChoice {
        PagesChoice::ALL.get(self.pages).copied().unwrap_or_default()
    }

    /// The resolution picked, DPI.
    fn dpi(&self) -> u32 {
        DPI_CHOICES.get(self.dpi).copied().unwrap_or(START_DPI)
    }
}

impl WordForm for ExportImagesForm {
    fn key(&self) -> &'static str {
        "exportImages"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Export as Images"))
    }

    fn ok_label(&self) -> Option<Label> {
        Some(Label::from(tl!("Export…")))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let rows: Vec<Label> = DPI_CHOICES.iter().map(|d| Label::from(crate::i18n::fmt(tl!("{dpi} DPI"), &[("dpi", &d.to_string())]))).collect();
        if let Some(at) = select_index(ui, &Label::from(tl!("Resolution:")), &rows, Some(self.dpi)) {
            self.dpi = at;
        }
        RadioGroup::new(Label::from(tl!("Pages:")), Bind::bound(&mut self.pages)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, cx| {
                for (at, c) in PagesChoice::ALL.iter().enumerate() {
                    cx.labelled(ui, at, Label::from(tl!(c.label()).to_owned()), SideLabel::new().layout(FieldLayout::After), Radio::new(at));
                }
            });
        });
        if self.choice() == PagesChoice::Custom {
            text_field(ui, "exportImagesPages", &Label::from(tl!("Pages to export")), &mut self.pages_text);
        }
        Text::new(Label::from(tl!("Each page is saved as its own PNG file. Several pages are numbered: name-1.png, name-2.png…"))).show(ui);
        self.choice() != PagesChoice::Custom || !self.pages_text.trim().is_empty()
    }

    /// A page range that names no page of the document is refused here, before the save dialog.
    fn check(&self, app: &WordApp) -> Result<(), Label> {
        if self.choice() != PagesChoice::Custom {
            return Ok(());
        }
        let total = app.session.export_layout().pages.len();
        wordcraft_engine::print::parse_page_range(&self.pages_text, total).map(|_| ()).map_err(|e| Label::from(sentence(&e)))
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let mut params = json!({"dpi": self.dpi(), "range": self.choice().range()});
        if self.choice() == PagesChoice::Custom {
            params["pages"] = json!(self.pages_text.trim());
        }
        vec![("file.exportPng".to_string(), params)]
    }
}

/// An engine message (`page 9 doesn't exist; …`) as a sentence, translated where the catalog has it.
fn sentence(message: &str) -> String {
    let message = tl!(message).to_string();
    let mut c = message.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect::<String>() + ".").unwrap_or_default()
}

/// Opens Export as Images.
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    Some(open_form(app, parent, ExportImagesForm::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_runs_the_export_with_the_resolution_and_pages_picked() {
        let mut f = ExportImagesForm::default();
        assert_eq!(f.dpi(), 300, "print resolution to start");
        f.dpi = DPI_CHOICES.iter().position(|&d| d == 1500).unwrap();
        f.pages = 0;
        assert_eq!(f.commands(), vec![("file.exportPng".to_string(), json!({"dpi": 1500, "range": "all"}))]);
        f.pages = 2;
        f.pages_text = " 2-3 ".into();
        assert_eq!(f.commands(), vec![("file.exportPng".to_string(), json!({"dpi": 1500, "range": "custom", "pages": "2-3"}))]);
    }

    #[test]
    fn a_range_outside_the_document_is_refused_before_the_save_dialog() {
        let app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("One page")), crate::Services::default());
        let mut f = ExportImagesForm { pages: 2, pages_text: "2".into(), ..Default::default() };
        assert!(f.check(&app).is_err());
        f.pages_text = "1".into();
        assert!(f.check(&app).is_ok());
        f.pages = 0;
        f.pages_text.clear();
        assert!(f.check(&app).is_ok(), "only typed pages are checked");
    }
}

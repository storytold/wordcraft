//! Save as Picture (#7743): the resolution to save the selected picture at, before asking where.
//! OK runs `picture.saveAs` with it and no path; the engine asks the app for the file, and the save
//! after that dialog keeps the choice. Scripts and agents pass `dpi` to `picture.saveAs` themselves.

use craft::ui::contract::Show as _;
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::runtime::env::Label;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wordcraft_engine::cmd::objects::save_picture::DPI_CHOICES;

use super::dialogs_insert::select_index;
use super::host::{Asks, WordForm, open_form};
use super::{Value, WordApp};

/// The Save as Picture form: the resolution picked, 0 for the picture's own, else the index into
/// [`DPI_CHOICES`] plus one.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavePictureForm {
    resolution: usize,
}

impl SavePictureForm {
    /// `picture.saveAs`'s `dpi` for the resolution picked.
    fn dpi(&self) -> Value {
        match self.resolution.checked_sub(1).and_then(|at| DPI_CHOICES.get(at)) {
            Some(d) => json!(d),
            None => json!("original"),
        }
    }
}

impl WordForm for SavePictureForm {
    fn key(&self) -> &'static str {
        "savePicture"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Save as Picture"))
    }

    fn ok_label(&self) -> Option<Label> {
        Some(Label::from(tl!("Save…")))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let mut rows = vec![Label::from(tl!("The picture's own"))];
        rows.extend(DPI_CHOICES.iter().map(|d| Label::from(crate::i18n::fmt(tl!("{dpi} DPI"), &[("dpi", &d.to_string())]))));
        if let Some(at) = select_index(ui, &Label::from(tl!("Resolution:")), &rows, Some(self.resolution)) {
            self.resolution = at;
        }
        let hint = if self.resolution == 0 {
            "Saves the picture's own pixels, cropped and adjusted as it is shown."
        } else {
            "Draws the picture at its size in the document with this many dots per inch."
        };
        Text::new(Label::from(tl!(hint))).show(ui);
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("picture.saveAs".to_string(), json!({"dpi": self.dpi()}))]
    }
}

/// Opens Save as Picture.
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    Some(open_form(app, parent, SavePictureForm::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_runs_save_as_picture_with_the_resolution_picked() {
        let mut f = SavePictureForm::default();
        assert_eq!(f.commands(), vec![("picture.saveAs".to_string(), json!({"dpi": "original"}))]);
        f.resolution = DPI_CHOICES.iter().position(|&d| d == 1500).unwrap() + 1;
        assert_eq!(f.commands(), vec![("picture.saveAs".to_string(), json!({"dpi": 1500}))]);
    }
}

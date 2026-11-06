//! Design › Watermark › Custom Watermark… (#2954): no watermark, a picture watermark (Select
//! Picture…, Scale, Washout) or a text watermark (Text, Semitransparent with a transparency
//! percentage, Layout). OK runs `design.watermark`, so scripts and agents get the same result
//! without the dialog. Shown through [`crate::dialogs::Dialog::Watermark`].

use egui::{Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};

use crate::WordApp;
use crate::dialogs::buttons;
use crate::file_dialogs::{AfterPick, FileDialogRequest};

/// Word's Scale choices for a picture watermark, percent; None is Auto.
const SCALES: [Option<f32>; 6] = [None, Some(500.0), Some(200.0), Some(150.0), Some(100.0), Some(50.0)];

/// The Custom Watermark dialog's fields.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkForm {
    /// `none`, `picture` or `text`.
    pub kind: String,
    /// A picture file picked with Select Picture….
    pub path: Option<String>,
    /// The picture of the document's picture watermark, kept when no other is picked.
    pub media: Option<String>,
    /// Percent of the picture's own size; None for Auto.
    pub scale: Option<f32>,
    pub washout: bool,
    pub text: String,
    pub semitransparent: bool,
    /// Percent, 0 to 100; Semitransparent is 50.
    pub transparency: u8,
    pub diagonal: bool,
    /// Why OK didn't apply.
    pub message: String,
}

impl WatermarkForm {
    /// The document's watermark; with none, No watermark and Word's defaults.
    pub fn read(app: &mut WordApp) -> WatermarkForm {
        // A file picked for a dialog that has since closed doesn't belong to this one.
        app.dialog_pick = None;
        let wm = app.session.doc.settings.watermark.clone();
        let pic = wm.as_ref().and_then(|w| w.picture.clone());
        let text = wm.clone().unwrap_or_else(|| wordcraft_doc::Watermark { text: "CONFIDENTIAL".into(), ..Default::default() });
        let kind = match &wm {
            None => "none",
            Some(w) if w.picture.is_some() => "picture",
            Some(_) => "text",
        };
        WatermarkForm {
            kind: kind.into(),
            path: None,
            media: pic.as_ref().map(|p| p.media.clone()),
            scale: pic.as_ref().and_then(|p| p.scale),
            washout: pic.as_ref().is_none_or(|p| p.washout),
            semitransparent: text.transparency() > 0,
            transparency: text.transparency(),
            diagonal: text.diagonal,
            text: text.text,
            message: String::new(),
        }
    }

    /// `design.watermark` parameters; None for a picture watermark without a picture.
    pub fn params(&self) -> Option<Value> {
        match self.kind.as_str() {
            "none" => Some(json!({"remove": true})),
            "picture" => {
                let picture = match (&self.path, &self.media) {
                    (Some(p), _) => json!({"path": p}),
                    (None, Some(m)) => json!({"media": m}),
                    (None, None) => return None,
                };
                let scale = self.scale.map_or(json!("auto"), |s| json!(s));
                Some(json!({"picture": picture, "scale": scale, "washout": self.washout}))
            }
            _ => Some(json!({"text": self.text, "diagonal": self.diagonal, "transparency": self.transparency})),
        }
    }

    /// The picture's name: the picked file's, or the document's picture.
    fn picture_name(&self) -> String {
        match (&self.path, &self.media) {
            (Some(p), _) => std::path::Path::new(p).file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned()),
            (None, Some(m)) => m.clone(),
            (None, None) => String::new(),
        }
    }
}

/// A Scale choice as shown: Auto, or the percentage.
fn scale_label(s: Option<f32>) -> String {
    match s {
        None => tl!("Auto").to_string(),
        Some(p) => format!("{p:.0}%"),
    }
}

/// Returns true to close.
pub fn watermark(app: &mut WordApp, ui: &mut Ui, f: &mut WatermarkForm) -> bool {
    if let Some(p) = app.dialog_pick.take() {
        f.path = Some(p);
        f.kind = "picture".into();
        f.message.clear();
    }
    ui.radio_value(&mut f.kind, "none".to_string(), tl!("No watermark"));
    ui.radio_value(&mut f.kind, "picture".to_string(), tl!("Picture watermark"));
    ui.add_enabled_ui(f.kind == "picture", |ui| {
        ui.indent("wm_picture", |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(crate::dialogs_templates::can_pick(app), egui::Button::new(tl!("Select Picture…"))).clicked() {
                    let _ = app.ask_file(FileDialogRequest::Open { purpose: "picture".into() }, AfterPick::ForDialog);
                }
                ui.label(f.picture_name());
            });
            ui.horizontal(|ui| {
                let label = ui.label(tl!("Scale:"));
                let combo = craft_ui::ComboBox::from_id_salt("wm_scale").width(80.0).selected_text(scale_label(f.scale)).show_ui(ui, |ui| {
                    for s in SCALES {
                        ui.selectable_value(&mut f.scale, s, scale_label(s));
                    }
                });
                combo.response.labelled_by(label.id);
                ui.add_space(16.0);
                ui.checkbox(&mut f.washout, tl!("Washout"));
            });
        });
    });
    ui.radio_value(&mut f.kind, "text".to_string(), tl!("Text watermark"));
    ui.add_enabled_ui(f.kind == "text", |ui| {
        ui.indent("wm_text", |ui| {
            crate::rtl::grid(ui, egui::Grid::new("wm_text_grid").num_columns(2).spacing(vec2(10.0, 6.0)), |ui| {
                let label = ui.label(tl!("Text:"));
                ui.add(egui::TextEdit::singleline(&mut f.text).desired_width(220.0).char_limit(200)).labelled_by(label.id);
                ui.end_row();
                ui.label(tl!("Transparency:"));
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut f.semitransparent, tl!("Semitransparent")).changed() {
                        f.transparency = if f.semitransparent { 50 } else { 0 };
                    }
                    if ui.add(egui::DragValue::new(&mut f.transparency).range(0..=100).suffix(" %")).changed() {
                        f.semitransparent = f.transparency > 0;
                    }
                });
                ui.end_row();
                ui.label(tl!("Layout:"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut f.diagonal, true, tl!("Diagonal"));
                    ui.radio_value(&mut f.diagonal, false, tl!("Horizontal"));
                });
                ui.end_row();
            });
        });
    });
    if !f.message.is_empty() {
        ui.colored_label(ui.visuals().error_fg_color, f.message.as_str());
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let Some(params) = f.params() else {
            f.message = tl!("Select a picture first.").to_string();
            return false;
        };
        match app.run("design.watermark", params) {
            Ok(_) => return true,
            Err(e) => f.message = e,
        }
    }
    cancel
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::*;
    use crate::dialogs::Dialog;
    use crate::{Services, WordApp};

    fn png_file(name: &str) -> String {
        let path = std::env::temp_dir().join(format!("wordcraft-wm-{}-{name}.png", std::process::id()));
        image::RgbaImage::from_pixel(60, 30, image::Rgba([20, 90, 160, 255])).save(&path).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        )
    }

    fn form(a: &WordApp) -> WatermarkForm {
        let Some(Dialog::Watermark { form }) = &a.dialog else { panic!("no watermark dialog: {:?}", a.dialog) };
        (**form).clone()
    }

    /// #2954: Select Picture… puts the picked picture in the dialog and OK makes it the
    /// watermark, washed out at Auto scale; reopened, the dialog shows it.
    #[test]
    fn dialog_sets_a_picture_watermark() {
        let path = png_file("pick");
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let picked = path.clone();
        a.services.pick_open = Some(Box::new(move |purpose| (purpose == "picture").then(|| picked.clone())));
        a.run("ui.dialog", json!({"name": "watermark"})).unwrap();
        assert_eq!(form(&a).kind, "none", "no watermark yet");
        let mut h = harness(a);
        h.run_steps(3);
        h.get_by_label("Picture watermark").click();
        h.run_steps(2);
        h.get_by_label("Select Picture…").click();
        h.run_steps(3);
        let f = form(h.state());
        assert_eq!((f.kind.as_str(), f.path.as_deref()), ("picture", Some(path.as_str())));
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().dialog.is_none());
        let wm = h.state().session.doc.settings.watermark.clone().unwrap();
        let pic = wm.picture.unwrap();
        assert_eq!((pic.scale, pic.washout), (None, true));
        h.state_mut().run("ui.dialog", json!({"name": "watermark"})).unwrap();
        let f = form(h.state());
        assert_eq!((f.kind.as_str(), f.media.as_deref(), f.path), ("picture", Some(pic.media.as_str()), None));
        let _ = std::fs::remove_file(&path);
    }

    /// #2954: the Semitransparent box and the transparency percentage follow each other, and
    /// OK sets the text watermark's transparency.
    #[test]
    fn dialog_sets_text_transparency() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("design.watermark", json!({"text": "DRAFT"})).unwrap();
        a.run("ui.dialog", json!({"name": "watermark"})).unwrap();
        let f = form(&a);
        assert_eq!((f.kind.as_str(), f.semitransparent, f.transparency), ("text", true, 50));
        let mut h = harness(a);
        h.run_steps(3);
        h.get_by_label("Semitransparent").click();
        h.run_steps(2);
        assert_eq!((form(h.state()).semitransparent, form(h.state()).transparency), (false, 0));
        if let Some(Dialog::Watermark { form }) = &mut h.state_mut().dialog {
            form.transparency = 75;
        }
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        let wm = h.state().session.doc.settings.watermark.clone().unwrap();
        assert_eq!((wm.transparency(), wm.picture), (75, None));
        // OK with No watermark removes it.
        h.state_mut().run("ui.dialog", json!({"name": "watermark"})).unwrap();
        h.run_steps(3);
        h.get_by_label("No watermark").click();
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert_eq!(h.state().session.doc.settings.watermark, None);
    }

    /// #2954: a picture watermark without a picture, or with one that can't be read, keeps the
    /// dialog open with a message and changes nothing.
    #[test]
    fn dialog_needs_a_readable_picture() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.dialog", json!({"name": "watermark"})).unwrap();
        let mut f = form(&a);
        f.kind = "picture".into();
        assert_eq!(f.params(), None);
        f.path = Some("/nonexistent/wm.png".into());
        assert!(a.run("design.watermark", f.params().unwrap()).is_err());
        assert_eq!(a.session.doc.settings.watermark, None);
        let mut h = harness(a);
        h.run_steps(3);
        h.get_by_label("Picture watermark").click();
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert_eq!(form(h.state()).message, "Select a picture first.");
        assert_eq!(h.state().session.doc.settings.watermark, None);
    }
}

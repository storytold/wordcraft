use super::common::{BUILT_IN_LABELS, NUMBER_FORMATS, SEPARATORS, choice};
use super::shell::{Next, go};
use super::*;

/// The Caption dialog's fields.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionForm {
    pub label: String,
    /// Built-in labels and the user's own (kept with the interface settings, like Word's
    /// Normal template keeps them).
    pub labels: Vec<String>,
    pub text: String,
    /// `below` or `above`.
    pub position: String,
    pub exclude_label: bool,
    /// `\*` format: `ARABIC`, `alphabetic`, `ALPHABETIC`, `roman`, `ROMAN`.
    pub format: String,
    pub chapter: bool,
    /// Heading level that starts a chapter (1–9).
    pub chapter_level: u8,
    pub separator: String,
    /// New Label's text.
    pub new_label: String,
}

impl CaptionForm {
    pub fn read(app: &WordApp) -> CaptionForm {
        let mut labels: Vec<String> = BUILT_IN_LABELS.iter().map(ToString::to_string).collect();
        for l in &app.ui.caption_labels {
            if !labels.contains(l) {
                labels.push(l.clone());
            }
        }
        // In a table the caption is for the table, unless a picture or shape in it is selected.
        let in_table = wordcraft_engine::cmd::objects::object_selection(&app.session).is_none()
            && app.session.sel.focus.path.depth() > 0
            && app
                .session
                .doc
                .body
                .get(app.session.sel.focus.path.0.first().copied().unwrap_or(0) as usize)
                .is_some_and(|b| matches!(&**b, Block::Table(_)));
        let last = &app.ui.last_caption_label;
        let label = if labels.contains(last) { last.as_str() } else { "Figure" };
        CaptionForm {
            label: if in_table { "Table" } else { label }.into(),
            labels,
            text: String::new(),
            position: "below".into(),
            exclude_label: false,
            format: "ARABIC".into(),
            chapter: false,
            chapter_level: 1,
            separator: "-".into(),
            new_label: String::new(),
        }
    }

    /// The number as it will show (`Figure 1-A`).
    pub fn preview(&self) -> String {
        let n = match self.format.as_str() {
            "alphabetic" => "a",
            "ALPHABETIC" => "A",
            "roman" => "i",
            "ROMAN" => "I",
            _ => "1",
        };
        let chapter = if self.chapter { format!("1{}", self.separator) } else { String::new() };
        let label = if self.exclude_label { String::new() } else { format!("{} ", self.label) };
        format!("{label}{chapter}{n}")
    }

    /// `references.caption` parameters.
    pub fn params(&self) -> Value {
        let mut v = json!({
            "label": self.label,
            "text": self.text.trim(),
            "position": self.position,
            "excludeLabel": self.exclude_label,
            "format": self.format,
        });
        if self.chapter {
            v["chapter"] = json!(self.chapter_level);
            v["separator"] = json!(self.separator);
        }
        v
    }

    /// Insert the caption and remember its label for next time.
    pub fn insert(&self, app: &mut WordApp) -> Result<(), String> {
        app.run("references.caption", self.params())?;
        app.ui.last_caption_label = self.label.clone();
        Ok(())
    }
}

/// Draws the Caption dialog's preview, text, label, position and numbering controls, with New
/// Label, Delete Label and Numbering jumps, and inserts the caption on OK.
pub(super) fn caption_ui(app: &mut WordApp, ui: &mut Ui, f: &mut CaptionForm) -> Next {
    let caption_label = ui.label(tl!("Caption:"));
    ui.horizontal(|ui| {
        ui.label(f.preview());
        ui.add(egui::TextEdit::singleline(&mut f.text).desired_width(220.0).hint_text(tl!("Caption text"))).labelled_by(caption_label.id);
    });
    ui.add_space(6.0);
    let mut then = None;
    egui::Grid::new("cap_opts").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let label_label = ui.label(tl!("Label:"));
        let labels: Vec<(String, &str)> = f.labels.iter().map(|l| (l.clone(), l.as_str())).collect();
        let combo = craft_ui::ComboBox::from_id_salt("cap_label").selected_text(tl!(&f.label)).width(150.0).show_ui(ui, |ui| {
            for (v, l) in &labels {
                ui.selectable_value(&mut f.label, v.clone(), tl!(l));
            }
        });
        combo.response.labelled_by(label_label.id);
        ui.end_row();
        let position_label = ui.label(tl!("Position:"));
        choice(ui, "cap_pos", &mut f.position, &[("below".to_string(), "Below the item"), ("above".to_string(), "Above the item")], &position_label);
        ui.end_row();
    });
    ui.checkbox(&mut f.exclude_label, tl!("Leave the label out of the caption"));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button(tl!("New Label…")).clicked() {
            let mut n = f.clone();
            n.new_label.clear();
            then = Some(RefsDialog::NewLabel(n));
        }
        let custom = !BUILT_IN_LABELS.contains(&f.label.as_str());
        if ui.add_enabled(custom, egui::Button::new(tl!("Delete Label"))).clicked() {
            app.ui.caption_labels.retain(|l| *l != f.label);
            f.labels.retain(|l| *l != f.label);
            f.label = "Figure".into();
        }
        if ui.button(tl!("Numbering…")).clicked() {
            then = Some(RefsDialog::CaptionNumbering { was: Box::new(f.clone()), form: f.clone() });
        }
    });
    if let Some(g) = then {
        return go(g);
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && let Err(e) = f.insert(app) {
        app.status(e);
    }
    if ok || cancel { Next::Close } else { Next::Stay }
}

/// Draws the caption numbering format, chapter-number toggle and chapter level and separator
/// controls, returning to the caption dialog with the edited or original form.
pub(super) fn numbering_ui(ui: &mut Ui, f: &mut CaptionForm, was: &CaptionForm) -> Next {
    egui::Grid::new("cap_num").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let format_label = ui.label(tl!("Format:"));
        let opts: Vec<(String, &str)> = NUMBER_FORMATS.iter().map(|(v, l)| (v.to_string(), *l)).collect();
        choice(ui, "cap_fmt", &mut f.format, &opts, &format_label);
        ui.end_row();
    });
    ui.checkbox(&mut f.chapter, tl!("Add the chapter number"));
    ui.add_enabled_ui(f.chapter, |ui| {
        egui::Grid::new("cap_chap").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            let chapter_label = ui.label(tl!("Chapters begin at:"));
            let opts: Vec<(u8, String)> = (1..=9u8).map(|l| (l, crate::i18n::fmt(tl!("Heading {n}"), &[("n", &l.to_string())]))).collect();
            let shown = opts.iter().find(|(l, _)| *l == f.chapter_level).map(|(_, s)| s.clone()).unwrap_or_default();
            let combo = craft_ui::ComboBox::from_id_salt("cap_level").selected_text(shown).width(150.0).show_ui(ui, |ui| {
                for (l, s) in &opts {
                    ui.selectable_value(&mut f.chapter_level, *l, s);
                }
            });
            combo.response.labelled_by(chapter_label.id);
            ui.end_row();
            let separator_label = ui.label(tl!("Separator:"));
            let opts: Vec<(String, &str)> = SEPARATORS.iter().map(|(v, l)| (v.to_string(), *l)).collect();
            choice(ui, "cap_sep", &mut f.separator, &opts, &separator_label);
            ui.end_row();
        });
    });
    ui.add_space(4.0);
    ui.label(crate::i18n::fmt(tl!("Example: {sample}"), &[("sample", &f.preview())]));
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        return go(RefsDialog::Caption(f.clone()));
    }
    if cancel {
        return go(RefsDialog::Caption(was.clone()));
    }
    Next::Stay
}

/// Draws the new caption label field with focus, sanitizes an accepted label, remembers custom
/// labels, and returns to the caption dialog.
pub(super) fn new_label_ui(app: &mut WordApp, ui: &mut Ui, f: &mut CaptionForm) -> Next {
    let label = ui.label(tl!("Label:"));
    let r = ui.add(egui::TextEdit::singleline(&mut f.new_label).desired_width(260.0)).labelled_by(label.id);
    if !r.has_focus() {
        r.request_focus();
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    let name: String = f.new_label.chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).take(40).collect::<String>().trim().to_string();
    if ok && !name.is_empty() {
        if !f.labels.contains(&name) {
            f.labels.push(name.clone());
        }
        if !BUILT_IN_LABELS.contains(&name.as_str()) && !app.ui.caption_labels.contains(&name) {
            app.ui.caption_labels.push(name.clone());
        }
        f.label = name;
    }
    if ok || cancel { go(RefsDialog::Caption(f.clone())) } else { Next::Stay }
}

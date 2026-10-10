//! Footnote and Endnote (References › Footnotes launcher, #385): where notes go, how they are
//! numbered, for the whole document or the caret's section; Convert; Insert with a custom mark.
//! The form ends in commands (`references.noteOptions`, `references.convertNotes`,
//! `references.footnote` / `references.endnote`), so agents get the same result without it.

use egui::{Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};

use crate::WordApp;
use crate::theme::semibold;

/// Number formats notes offer: OOXML name and a sample (not translated).
const FORMATS: [(&str, &str); 5] = [
    ("decimal", "1, 2, 3, …"),
    ("lowerLetter", "a, b, c, …"),
    ("upperLetter", "A, B, C, …"),
    ("lowerRoman", "i, ii, iii, …"),
    ("upperRoman", "I, II, III, …"),
];

/// The dialog's fields; `[footnotes, endnotes]` where each kind has its own.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NoteForm {
    /// Which kind the format fields and Insert are for.
    pub endnote: bool,
    pub pos: [String; 2],
    pub fmt: [String; 2],
    pub start: [u32; 2],
    pub restart: [String; 2],
    /// A custom mark for Insert (empty: numbered).
    pub mark: String,
    /// Apply to the whole document (else the caret's section).
    pub whole: bool,
}

impl NoteForm {
    /// The options in effect at the caret.
    pub fn read(app: &mut WordApp) -> NoteForm {
        let v = app.session.run("references.noteOptions", &json!({})).unwrap_or_default();
        let s = |kind: &str, k: &str, d: &str| v.get(kind).and_then(|o| o.get(k)).and_then(Value::as_str).unwrap_or(d).to_string();
        let n = |kind: &str| v.get(kind).and_then(|o| o.get("numStart")).and_then(Value::as_u64).unwrap_or(1).clamp(1, 32_767) as u32;
        NoteForm {
            endnote: false,
            pos: [s("footnote", "pos", "pageBottom"), s("endnote", "pos", "docEnd")],
            fmt: [s("footnote", "numFmt", "decimal"), s("endnote", "numFmt", "lowerRoman")],
            start: [n("footnote"), n("endnote")],
            restart: [s("footnote", "numRestart", "continuous"), s("endnote", "numRestart", "continuous")],
            mark: String::new(),
            whole: true,
        }
    }

    /// `references.noteOptions` parameters for the chosen kind.
    pub fn params(&self) -> Value {
        let k = usize::from(self.endnote);
        let get = |a: &[String; 2]| a.get(k).cloned().unwrap_or_default();
        json!({
            "kind": if self.endnote { "endnote" } else { "footnote" },
            "pos": get(&self.pos),
            "numFmt": get(&self.fmt),
            "numStart": self.start.get(k).copied().unwrap_or(1),
            "numRestart": get(&self.restart),
            "scope": if self.whole { "document" } else { "section" },
        })
    }
}

/// A choice from `options` ((value, label)) in a drop-down; `tr` translates the labels.
fn choice(ui: &mut Ui, id: &str, value: &mut String, options: &[(&str, &str)], tr: bool) {
    let show = |l: &str| if tr { tl!(l).to_string() } else { l.to_string() };
    let label = options.iter().find(|(v, _)| *v == value.as_str()).map(|(_, l)| show(l)).unwrap_or_default();
    egui::ComboBox::from_id_salt(id).selected_text(label).width(170.0).show_ui(ui, |ui| {
        for (v, l) in options {
            ui.selectable_value(value, v.to_string(), show(l));
        }
    });
}

/// Footnote and Endnote. Returns true to close.
pub fn note_options(app: &mut WordApp, ui: &mut Ui, f: &mut NoteForm) -> bool {
    let heading = |ui: &mut Ui, s: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
    };
    heading(ui, "Location");
    let mut convert = None;
    egui::Grid::new("notes_location").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.radio_value(&mut f.endnote, false, tl!("Footnotes"));
        ui.add_enabled_ui(!f.endnote, |ui| {
            choice(ui, "notes_fpos", &mut f.pos[0], &[("pageBottom", "Bottom of Page"), ("beneathText", "Below text")], true);
        });
        ui.end_row();
        ui.radio_value(&mut f.endnote, true, tl!("Endnotes"));
        ui.add_enabled_ui(f.endnote, |ui| {
            choice(ui, "notes_epos", &mut f.pos[1], &[("sectEnd", "End of section"), ("docEnd", "End of document")], true);
        });
        ui.end_row();
        ui.label("");
        ui.menu_button(format!("{}…", tl!("Convert")), |ui| {
            for (to, l) in [("endnote", "Footnotes to endnotes"), ("footnote", "Endnotes to footnotes"), ("swap", "Swap footnotes and endnotes")] {
                if ui.button(tl!(l)).clicked() {
                    convert = Some(to);
                    ui.close();
                }
            }
        });
        ui.end_row();
    });
    if let Some(to) = convert
        && let Err(e) = app.run("references.convertNotes", json!({ "to": to }))
    {
        app.status(e);
    }
    heading(ui, "Format");
    let k = usize::from(f.endnote);
    egui::Grid::new("notes_format").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Number format"));
        if let Some(v) = f.fmt.get_mut(k) {
            choice(ui, "notes_fmt", v, &FORMATS, false);
        }
        ui.end_row();
        ui.label(tl!("Custom mark:"));
        ui.add(egui::TextEdit::singleline(&mut f.mark).desired_width(60.0).char_limit(10));
        ui.end_row();
        ui.label(tl!("Start at:"));
        if let Some(v) = f.start.get_mut(k) {
            ui.add(egui::DragValue::new(v).range(1..=32_767).speed(0.2));
        }
        ui.end_row();
        ui.label(tl!("Numbering"));
        if let Some(v) = f.restart.get_mut(k) {
            let each_page: &[(&str, &str)] = if f.endnote { &[] } else { &[("eachPage", "Restart Each Page")] };
            let options: Vec<(&str, &str)> =
                [("continuous", "Continuous numbering"), ("eachSect", "Restart Each Section")].into_iter().chain(each_page.iter().copied()).collect();
            choice(ui, "notes_restart", v, &options, true);
        }
        ui.end_row();
        ui.label(tl!("Apply to:"));
        let mut scope = if f.whole { "document" } else { "section" }.to_string();
        choice(ui, "notes_scope", &mut scope, &[("document", "Whole document"), ("section", "This section")], true);
        f.whole = scope == "document";
        ui.end_row();
    });
    // Insert / Apply / Cancel.
    let (mut insert, mut apply, mut cancel) = (false, false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel = ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            apply = ui.button(tl!("Apply")).clicked();
            let label = if f.endnote { "Insert Endnote" } else { "Insert Footnote" };
            insert = ui.add(egui::Button::new(egui::RichText::new(tl!(label)).color(egui::Color32::WHITE)).fill(crate::theme::APP_COLOR)).clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
    });
    if insert || apply {
        if let Err(e) = app.run("references.noteOptions", f.params()) {
            app.status(e);
            return false;
        }
        if insert {
            let id = if f.endnote { "references.endnote" } else { "references.footnote" };
            if let Err(e) = app.run(id, json!({ "mark": f.mark.trim() })) {
                app.status(e);
            }
        }
    }
    insert || apply || cancel
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use crate::dialogs::Dialog;
    use crate::{Services, WordApp};

    /// #385: the Footnotes group's launcher opens Footnote and Endnote; its settings apply to the
    /// caret's section. Scripts get the options, never the dialog.
    #[test]
    fn footnote_and_endnote_opens_from_the_launcher_and_applies() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        assert_eq!(a.run("references.noteOptions", json!({})).unwrap(), json!({"pending": "noteOptions"}));
        let Some(Dialog::NoteOptions { form }) = &mut a.dialog else { panic!("dialog") };
        assert_eq!((form.pos[0].as_str(), form.fmt[1].as_str(), form.start[0]), ("pageBottom", "lowerRoman", 1));
        form.pos[0] = "beneathText".into();
        form.restart[0] = "eachPage".into();
        form.whole = false;
        let params = form.params();
        a.dialog = None;
        a.run("references.noteOptions", params).unwrap();
        let o = a.session.doc.note_options(&a.session.doc.last_section, false);
        assert_eq!(o.pos, wordcraft_doc::section::NotePos::BeneathText);
        assert!(a.session.doc.settings.footnote_pr.is_empty(), "this section only");
        let got = a.execute("references.noteOptions", json!({})).unwrap();
        assert_eq!(got["footnote"]["numRestart"], "eachPage");
        assert!(a.dialog.is_none());
    }
}
